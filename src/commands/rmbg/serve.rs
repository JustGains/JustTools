//! Resident `--serve` mode: load the model once, then answer one JSON request
//! per stdin line with exactly one JSON line on stdout, in request order.
//! Stdout is the protocol channel; human status goes to stderr only.
//!
//! Requests pass through three threads in order, so the model never waits for
//! image work: this thread reads and decodes the next image while the model
//! runs on the current one, and a writer thread saves the one before. Command-
//! line batches run through the same stages (`run_requests`), so both modes
//! make the same files.

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use image::{ImageFormat, ImageReader};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::commands::image_ops;
use crate::error::{ToolError, ToolResult};

use super::image_pipeline::{Format, OutputOptions, PngCompression, PreparedImage, Written};
use super::{Provider, TOOL, jobs};

/// Longest accepted request line. Paths alone stay far below this; the cap
/// keeps a runaway caller from growing one line without bound.
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// Requests that may wait between two threads. Each holds its decoded image,
/// so the queues stay short; a full queue pauses reading until the model
/// catches up.
const QUEUED_REQUESTS: usize = 2;

/// `defaults` are every request's output options (from the command line).
pub(super) fn run(
    provider: Provider,
    download: bool,
    model_override: Option<&Path>,
    full_precision: bool,
    defaults: &OutputOptions,
) -> ToolResult {
    let started = Instant::now();
    let mut session =
        super::Session::start(provider, download, model_override, full_precision, report)?;
    // The first inference prepares the GPU for this model and takes seconds;
    // running it now keeps that wait out of the first request.
    session.warm_up(report)?;
    eprintln!(
        "Model loaded in {:.1}s — selected provider: {}",
        started.elapsed().as_secs_f64(),
        session.label()
    );

    let mut stdout = io::stdout();
    write_line(
        &mut stdout,
        &Ready {
            ready: true,
            provider: session.provider_name(),
            model: session.model_name(),
            load_seconds: seconds(started.elapsed()),
            pictures: true,
        },
    )?;
    eprintln!("{TOOL}: serving; send one JSON request per stdin line");
    let protected = session.protected_paths();
    let size = session.size();
    let (engine, fallback) = session.parts();
    serve_lines(
        &mut io::stdin().lock(),
        &mut stdout,
        defaults,
        |request| {
            check_request(request, &protected)?;
            PreparedImage::load(&request.input, size)
                .map(|prepared| (prepared, absolute(&request.output), request.options.clone()))
        },
        |(prepared, output, options)| {
            // The engine is shared, so a GPU-to-CPU fallback lasts for every
            // later request too.
            let mask = super::infer_with_fallback(engine, fallback, &prepared.chw, report)?;
            Ok((prepared, mask, output, options))
        },
        |(prepared, mask, output, options)| {
            let written = prepared.write_with_mask(&mask, &output, &options)?;
            Ok((output, written))
        },
    )
}

#[derive(Serialize)]
struct Ready {
    ready: bool,
    provider: &'static str,
    model: String,
    load_seconds: f64,
    /// Requests may ask for a finished picture (cropped, padded, scaled, WebP).
    pictures: bool,
}

#[derive(Debug, PartialEq)]
pub(super) struct Request {
    pub id: String,
    pub input: PathBuf,
    pub output: PathBuf,
    pub options: OutputOptions,
}

/// Why a line is not a usable request; `id` is echoed when it was readable.
#[derive(Debug, PartialEq)]
pub(super) struct Rejected {
    pub id: Option<String>,
    pub message: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Response {
    Success {
        id: String,
        ok: bool,
        output: String,
        seconds: f64,
        stages: Stages,
        width: u32,
        height: u32,
        bytes: u64,
    },
    Failure {
        id: Option<String>,
        ok: bool,
        error: String,
    },
}

/// Seconds a request spent in each step: decoding and scaling (`prepare`),
/// waiting for the model behind earlier requests (`wait`), in the model
/// (`infer`), and making and saving the cut-out (`write`). Its `seconds` also
/// counts the time it waited to be written.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(super) struct Stages {
    pub prepare: f64,
    pub wait: f64,
    pub infer: f64,
    pub write: f64,
}

impl Response {
    fn success(
        id: String,
        output: &Path,
        elapsed: Duration,
        stages: Stages,
        written: Written,
    ) -> Self {
        Self::Success {
            id,
            ok: true,
            output: output.to_string_lossy().into_owned(),
            seconds: seconds(elapsed),
            stages,
            width: written.width,
            height: written.height,
            bytes: written.bytes,
        }
    }

    fn failure(id: Option<String>, error: impl Into<String>) -> Self {
        Self::Failure {
            id,
            ok: false,
            error: error.into(),
        }
    }
}

/// A request on its way through the threads: still being worked on, or answered.
enum Step<T> {
    Working {
        id: String,
        read: Instant,
        ready: Instant,
        stages: Stages,
        work: T,
    },
    Answered(Response),
}

/// Answers every non-blank line until end of input, in order. `prepare` runs
/// on this thread as each line is read, `infer` on a second thread and `write`
/// (which returns the absolute output path and what it wrote) on a third, so
/// three requests can be in progress at once.
fn serve_lines<R, W, P, Q>(
    input: &mut R,
    output: &mut W,
    defaults: &OutputOptions,
    prepare: impl FnMut(&Request) -> ToolResult<P>,
    infer: impl FnMut(P) -> ToolResult<Q> + Send,
    write: impl FnMut(Q) -> ToolResult<(PathBuf, Written)> + Send,
) -> ToolResult
where
    R: BufRead,
    W: Write + Send,
    P: Send,
    Q: Send,
{
    let mut failed = None;
    let requests = std::iter::from_fn(|| match next_line(input, defaults) {
        Ok(Some(line)) => Some(line),
        Ok(None) => None,
        Err(error) => {
            failed = Some(error);
            None
        }
    });
    run_requests(
        requests,
        |response| write_line(output, &response),
        prepare,
        infer,
        write,
    )?;
    failed.map_or(Ok(()), Err)
}

/// The next request line, parsed: `None` at end of input. Blank lines are
/// skipped; an oversized line is consumed and rejected.
fn next_line<R: BufRead>(
    input: &mut R,
    defaults: &OutputOptions,
) -> ToolResult<Option<Result<Request, Rejected>>> {
    let read_error = |error: io::Error| ToolError::new(TOOL, format!("cannot read stdin: {error}"));
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = input
            .by_ref()
            .take(MAX_REQUEST_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .map_err(read_error)?;
        if read == 0 {
            return Ok(None);
        }
        if line.len() > MAX_REQUEST_BYTES && line.last() != Some(&b'\n') {
            input.skip_until(b'\n').map_err(read_error)?;
            return Ok(Some(Err(Rejected {
                id: None,
                message: format!("request line exceeds {MAX_REQUEST_BYTES} bytes"),
            })));
        }
        let text = line.trim_ascii();
        if !text.is_empty() {
            return Ok(Some(parse_request(text, defaults)));
        }
    }
}

/// Runs requests through the three stages and hands each answer, in request
/// order, to `answer` (on the writer thread). Stops early only when `answer`
/// fails, and returns that error.
pub(super) fn run_requests<P, Q>(
    requests: impl Iterator<Item = Result<Request, Rejected>>,
    mut answer: impl FnMut(Response) -> ToolResult + Send,
    mut prepare: impl FnMut(&Request) -> ToolResult<P>,
    infer: impl FnMut(P) -> ToolResult<Q> + Send,
    write: impl FnMut(Q) -> ToolResult<(PathBuf, Written)> + Send,
) -> ToolResult
where
    P: Send,
    Q: Send,
{
    thread::scope(|scope| {
        let (to_model, model_queue) = mpsc::sync_channel(QUEUED_REQUESTS);
        let (to_writer, writer_queue) = mpsc::sync_channel(QUEUED_REQUESTS);
        let model = scope.spawn(move || run_model(model_queue, to_writer, infer));
        let writer = scope.spawn(move || write_answers(writer_queue, &mut answer, write));
        for request in requests {
            let step = match request {
                Ok(request) => {
                    let read = Instant::now();
                    match prepare(&request) {
                        Ok(work) => Step::Working {
                            id: request.id,
                            read,
                            ready: Instant::now(),
                            stages: Stages {
                                prepare: seconds(read.elapsed()),
                                ..Stages::default()
                            },
                            work,
                        },
                        Err(error) => {
                            Step::Answered(Response::failure(Some(request.id), error.message()))
                        }
                    }
                }
                Err(rejected) => Step::Answered(Response::failure(rejected.id, rejected.message)),
            };
            if to_model.send(step).is_err() {
                // The writer stopped and reports why.
                break;
            }
        }
        drop(to_model);
        let stopped = |_| ToolError::new(TOOL, "a serve thread stopped unexpectedly");
        model.join().map_err(stopped)?;
        writer.join().map_err(stopped)?
    })
}

fn run_model<P, Q>(
    queue: Receiver<Step<P>>,
    next: SyncSender<Step<Q>>,
    mut infer: impl FnMut(P) -> ToolResult<Q>,
) {
    for step in queue {
        let step = match step {
            Step::Working {
                id,
                read,
                ready,
                mut stages,
                work,
            } => {
                let started = Instant::now();
                stages.wait = seconds(started.duration_since(ready));
                match infer(work) {
                    Ok(work) => {
                        stages.infer = seconds(started.elapsed());
                        Step::Working {
                            id,
                            read,
                            ready,
                            stages,
                            work,
                        }
                    }
                    Err(error) => Step::Answered(Response::failure(Some(id), error.message())),
                }
            }
            Step::Answered(response) => Step::Answered(response),
        };
        if next.send(step).is_err() {
            return;
        }
    }
}

fn write_answers<Q>(
    queue: Receiver<Step<Q>>,
    answer: &mut impl FnMut(Response) -> ToolResult,
    mut write: impl FnMut(Q) -> ToolResult<(PathBuf, Written)>,
) -> ToolResult {
    for step in queue {
        let response = match step {
            Step::Working {
                id,
                read,
                mut stages,
                work,
                ..
            } => {
                let started = Instant::now();
                match write(work) {
                    Ok((path, written)) => {
                        stages.write = seconds(started.elapsed());
                        Response::success(id, &path, read.elapsed(), stages, written)
                    }
                    Err(error) => Response::failure(Some(id), error.message()),
                }
            }
            Step::Answered(response) => response,
        };
        answer(response)?;
    }
    Ok(())
}

fn parse_request(line: &[u8], defaults: &OutputOptions) -> Result<Request, Rejected> {
    let reject = |id: Option<&str>, message: String| Rejected {
        id: id.map(str::to_owned),
        message,
    };
    let value: Value = serde_json::from_slice(line)
        .map_err(|error| reject(None, format!("invalid JSON request: {error}")))?;
    let Value::Object(fields) = value else {
        return Err(reject(None, "request must be a JSON object".to_owned()));
    };
    let id = match fields.get("id") {
        Some(Value::String(id)) => id.as_str(),
        Some(_) => return Err(reject(None, "request \"id\" must be a string".to_owned())),
        None => return Err(reject(None, "request is missing \"id\"".to_owned())),
    };
    let path = |name: &str| match fields.get(name) {
        Some(Value::String(path)) if !path.is_empty() => Ok(PathBuf::from(path)),
        Some(Value::String(_)) => Err(reject(
            Some(id),
            format!("request \"{name}\" must not be empty"),
        )),
        Some(_) => Err(reject(
            Some(id),
            format!("request \"{name}\" must be a string"),
        )),
        None => Err(reject(Some(id), format!("request is missing \"{name}\""))),
    };
    Ok(Request {
        input: path("input")?,
        output: path("output")?,
        options: request_options(&fields, defaults).map_err(|message| reject(Some(id), message))?,
        id: id.to_owned(),
    })
}

/// A request's output options: the server's defaults, or with `picture`
/// (true, or an object of options) the finished-picture settings; then the
/// request's own fields. Fields are the command line's options in camelCase.
fn request_options(
    fields: &Map<String, Value>,
    defaults: &OutputOptions,
) -> Result<OutputOptions, String> {
    let mut options = defaults.clone();
    match fields.get("picture") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => options = OutputOptions::picture(),
        Some(Value::Object(picture)) => {
            options = OutputOptions::picture();
            apply_options(&mut options, picture, "picture.")?;
        }
        Some(_) => return Err("request \"picture\" must be true or an object".to_owned()),
    }
    apply_options(&mut options, fields, "")?;
    Ok(options)
}

fn apply_options(
    options: &mut OutputOptions,
    fields: &Map<String, Value>,
    prefix: &str,
) -> Result<(), String> {
    let number = |name: &str, low: u64, high: u64| -> Result<Option<u64>, String> {
        match fields.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .filter(|value| (low..=high).contains(value))
                .map(Some)
                .ok_or_else(|| {
                    format!(
                        "request \"{prefix}{name}\" must be a whole number from {low} to {high}"
                    )
                }),
        }
    };
    if let Some(value) = fields.get("format").filter(|value| !value.is_null()) {
        options.format =
            Some(value.as_str().and_then(Format::parse).ok_or_else(|| {
                format!("request \"{prefix}format\" must be \"png\" or \"webp\"")
            })?);
    }
    if let Some(value) = fields.get("crop").filter(|value| !value.is_null()) {
        options.crop = value
            .as_bool()
            .ok_or_else(|| format!("request \"{prefix}crop\" must be true or false"))?;
    }
    if let Some(value) = number("padding", 0, 10_000)? {
        options.padding = value as u32;
    }
    if let Some(value) = number("maxWidth", 1, 100_000)? {
        options.max_width = Some(value as u32);
    }
    if let Some(value) = number("maxHeight", 1, 100_000)? {
        options.max_height = Some(value as u32);
    }
    if let Some(value) = number("quality", 0, 100)? {
        options.quality = value as u8;
    }
    if let Some(value) = number("alphaQuality", 0, 100)? {
        options.alpha_quality = value as u8;
    }
    if let Some(value) = number("method", 0, 6)? {
        options.method = value as u8;
    }
    if let Some(value) = fields.get("compression").filter(|value| !value.is_null()) {
        options.png = match value.as_str() {
            Some("fast") => PngCompression::Fast,
            Some("small") => PngCompression::Small,
            _ => {
                return Err(format!(
                    "request \"{prefix}compression\" must be \"fast\" or \"small\""
                ));
            }
        };
    }
    Ok(())
}

/// Rejects requests that cannot or must not run, before any decoding, and
/// makes the output path absolute. `protected` are files that must never be
/// written over (the model files in use).
pub(super) fn check_request(request: &Request, protected: &[PathBuf]) -> ToolResult<()> {
    let input = &request.input;
    if !input.exists() {
        return Err(ToolError::new(
            TOOL,
            format!("input not found: {}", input.display()),
        ));
    }
    if !input.is_file() {
        return Err(ToolError::new(
            TOOL,
            format!("input is not a file: {}", input.display()),
        ));
    }
    image_ops::assert_static(TOOL, input)?;
    // assert_static cannot detect GIF animation, and normal runs never accept
    // GIF, so refuse it by content rather than silently using one frame.
    if sniffed_format(input) == Some(ImageFormat::Gif) {
        return Err(ToolError::new(
            TOOL,
            format!("unsupported image (GIF): {}", input.display()),
        ));
    }

    let output = std::path::absolute(&request.output)
        .map_err(|error| ToolError::new(TOOL, format!("invalid output path: {error}")))?;
    if output.is_dir() {
        return Err(ToolError::new(
            TOOL,
            format!("output is a directory: {}", output.display()),
        ));
    }
    if !output.parent().is_some_and(Path::is_dir) {
        return Err(ToolError::new(
            TOOL,
            format!(
                "output folder does not exist: {}",
                output.parent().unwrap_or(&output).display()
            ),
        ));
    }
    if jobs::same_path(input, &output)? {
        return Err(ToolError::new(
            TOOL,
            format!("output would overwrite the input: {}", output.display()),
        ));
    }
    let job = jobs::Job {
        input: input.clone(),
        output,
    };
    for model in protected {
        jobs::reject_model_overwrite(std::slice::from_ref(&job), model)?;
    }
    Ok(())
}

/// The absolute form of a request's output, as answers report it.
pub(super) fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn sniffed_format(path: &Path) -> Option<ImageFormat> {
    ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .format()
}

/// Writes one protocol line and flushes it so the caller sees it immediately.
fn write_line<W: Write>(output: &mut W, message: &impl Serialize) -> ToolResult {
    let mut line = serde_json::to_vec(message)
        .map_err(|error| ToolError::new(TOOL, format!("cannot encode response: {error}")))?;
    line.push(b'\n');
    output
        .write_all(&line)
        .and_then(|()| output.flush())
        .map_err(|error| ToolError::new(TOOL, format!("cannot write to stdout: {error}")))
}

fn report(message: &str) {
    eprintln!("{message}");
}

pub(super) fn seconds(elapsed: Duration) -> f64 {
    (elapsed.as_secs_f64() * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use super::*;

    fn line_of(message: &impl Serialize) -> String {
        let mut output = Vec::new();
        write_line(&mut output, message).unwrap();
        String::from_utf8(output).unwrap()
    }

    const WRITTEN: Written = Written {
        width: 3,
        height: 2,
        bytes: 99,
    };

    fn served(input: impl AsRef<[u8]>) -> (Vec<Value>, Vec<String>) {
        let mut output = Vec::new();
        let mut processed = Vec::new();
        serve_lines(
            &mut Cursor::new(input),
            &mut output,
            &OutputOptions::default(),
            |request| {
                processed.push(request.id.clone());
                if request.input.ends_with("missing.png") {
                    Err(ToolError::new(TOOL, "input not found: missing.png"))
                } else {
                    Ok(request.output.clone())
                }
            },
            |output: PathBuf| {
                if output.ends_with("unreadable-out.png") {
                    Err(ToolError::new(TOOL, "inference failed: test"))
                } else {
                    Ok(output)
                }
            },
            |output| Ok((PathBuf::from("/abs").join(output), WRITTEN)),
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.is_empty() || text.ends_with('\n'));
        let lines = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        (lines, processed)
    }

    #[test]
    fn parses_a_valid_request_and_ignores_extra_fields() {
        let request = parse_request(
            br#"{"id":"7","input":"C:\\frames\\in.webp","output":"out.png","note":1}"#,
            &OutputOptions::default(),
        )
        .unwrap();
        assert_eq!(
            request,
            Request {
                id: "7".into(),
                input: PathBuf::from(r"C:\frames\in.webp"),
                output: PathBuf::from("out.png"),
                options: OutputOptions::default(),
            }
        );
    }

    #[test]
    fn picture_requests_start_from_the_picture_settings() {
        let request = parse_request(
            br#"{"id":"p","input":"in.png","output":"out.webp","picture":{"maxHeight":300,"padding":4}}"#,
            &OutputOptions::default(),
        )
        .unwrap();
        assert_eq!(
            request.options,
            OutputOptions {
                max_height: Some(300),
                padding: 4,
                ..OutputOptions::picture()
            }
        );
        let plain = parse_request(
            br#"{"id":"p","input":"in.png","output":"out.webp","picture":true}"#,
            &OutputOptions::default(),
        )
        .unwrap();
        assert_eq!(plain.options, OutputOptions::picture());
        let own = parse_request(
            br#"{"id":"o","input":"in.png","output":"o.png","crop":true,"maxWidth":64,"format":"png","quality":90,"alphaQuality":100,"method":6,"compression":"small"}"#,
            &OutputOptions::default(),
        )
        .unwrap();
        assert_eq!(
            own.options,
            OutputOptions {
                format: Some(Format::Png),
                crop: true,
                max_width: Some(64),
                quality: 90,
                alpha_quality: 100,
                method: 6,
                png: PngCompression::Small,
                ..OutputOptions::default()
            }
        );
    }

    #[test]
    fn the_servers_defaults_apply_until_a_request_names_its_own() {
        let defaults = OutputOptions::picture();
        let request = parse_request(
            br#"{"id":"d","input":"a.png","output":"b.webp"}"#,
            &defaults,
        )
        .unwrap();
        assert_eq!(request.options, defaults);
        let request = parse_request(
            br#"{"id":"d","input":"a.png","output":"b.png","format":"png","crop":false,"maxHeight":null}"#,
            &defaults,
        )
        .unwrap();
        assert_eq!(request.options.format, Some(Format::Png));
        assert!(!request.options.crop);
        assert_eq!(request.options.max_height, Some(400));
    }

    #[test]
    fn bad_options_are_rejected_with_the_id() {
        for (line, field) in [
            (
                r#"{"id":"b","input":"a","output":"b","picture":"yes"}"#,
                "picture",
            ),
            (
                r#"{"id":"b","input":"a","output":"b","format":"gif"}"#,
                "format",
            ),
            (r#"{"id":"b","input":"a","output":"b","crop":1}"#, "crop"),
            (
                r#"{"id":"b","input":"a","output":"b","maxHeight":0}"#,
                "maxHeight",
            ),
            (
                r#"{"id":"b","input":"a","output":"b","quality":101}"#,
                "quality",
            ),
            (
                r#"{"id":"b","input":"a","output":"b","picture":{"method":9}}"#,
                "picture.method",
            ),
            (
                r#"{"id":"b","input":"a","output":"b","compression":"zip"}"#,
                "compression",
            ),
        ] {
            let rejected = parse_request(line.as_bytes(), &OutputOptions::default()).unwrap_err();
            assert_eq!(rejected.id.as_deref(), Some("b"), "{line}");
            assert!(rejected.message.contains(field), "{}", rejected.message);
        }
    }

    #[test]
    fn unreadable_requests_have_no_id() {
        for line in [
            "not json",
            r#"{"id":"1","input":"a.png""#,
            r#"["a.png","b.png"]"#,
            r#""a.png""#,
            r#"{"input":"a.png","output":"b.png"}"#,
            r#"{"id":5,"input":"a.png","output":"b.png"}"#,
        ] {
            let rejected = parse_request(line.as_bytes(), &OutputOptions::default()).unwrap_err();
            assert_eq!(rejected.id, None, "{line}");
            assert!(!rejected.message.is_empty());
        }
    }

    #[test]
    fn requests_with_bad_paths_echo_their_id() {
        for (line, field) in [
            (r#"{"id":"a1","output":"out.png"}"#, "input"),
            (r#"{"id":"a1","input":"in.png"}"#, "output"),
            (r#"{"id":"a1","input":"","output":"out.png"}"#, "input"),
            (r#"{"id":"a1","input":"in.png","output":7}"#, "output"),
        ] {
            let rejected = parse_request(line.as_bytes(), &OutputOptions::default()).unwrap_err();
            assert_eq!(rejected.id.as_deref(), Some("a1"), "{line}");
            assert!(rejected.message.contains(field), "{}", rejected.message);
        }
    }

    #[test]
    fn ready_line_matches_the_protocol() {
        let ready = Ready {
            ready: true,
            provider: "DirectML",
            model: "rmbg-2.0-fast-fp16".into(),
            load_seconds: seconds(Duration::from_millis(9_104)),
            pictures: true,
        };
        assert_eq!(
            line_of(&ready),
            "{\"ready\":true,\"provider\":\"DirectML\",\"model\":\"rmbg-2.0-fast-fp16\",\"load_seconds\":9.104,\"pictures\":true}\n"
        );
    }

    #[test]
    fn responses_escape_backslashes_and_quotes() {
        let path = r#"C:\cut "outs"\frame-0.png"#;
        let success = line_of(&Response::success(
            "x\"1".into(),
            Path::new(path),
            Duration::from_millis(923),
            Stages {
                prepare: 0.05,
                wait: 0.0,
                infer: 0.31,
                write: 0.02,
            },
            WRITTEN,
        ));
        assert_eq!(
            success,
            r#"{"id":"x\"1","ok":true,"output":"C:\\cut \"outs\"\\frame-0.png","seconds":0.923,"stages":{"prepare":0.05,"wait":0.0,"infer":0.31,"write":0.02},"width":3,"height":2,"bytes":99}"#
                .to_owned()
                + "\n"
        );
        let parsed: Value = serde_json::from_str(&success).unwrap();
        assert_eq!(parsed["output"], path);

        let failure = line_of(&Response::failure(None, "bad \\ \"line\"\nnext"));
        assert_eq!(
            failure,
            r#"{"id":null,"ok":false,"error":"bad \\ \"line\"\nnext"}"#.to_owned() + "\n"
        );
    }

    #[test]
    fn serving_answers_in_order_and_continues_after_bad_lines() {
        let (lines, processed) = served(concat!(
            "{\"id\":\"1\",\"input\":\"a.png\",\"output\":\"a-out.png\"}\n",
            "\n",
            "   \r\n",
            "not json\n",
            "{\"id\":\"2\",\"input\":\"missing.png\",\"output\":\"b-out.png\"}\r\n",
            "{\"id\":\"3\",\"input\":\"c.png\"}\n",
            "{\"id\":\"4\",\"input\":\"d.png\",\"output\":\"unreadable-out.png\"}\n",
            "{\"id\":\"5\",\"input\":\"e.png\",\"output\":\"e-out.png\"}",
        ));
        assert_eq!(processed, ["1", "2", "4", "5"]);
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0]["id"], "1");
        assert_eq!(lines[0]["ok"], true);
        assert!(lines[0]["output"].as_str().unwrap().ends_with("a-out.png"));
        assert!(lines[0]["seconds"].is_f64());
        assert!(lines[0]["stages"]["infer"].is_f64());
        assert!(lines[0]["stages"]["wait"].is_f64());
        assert_eq!(lines[0]["width"], 3);
        assert_eq!(lines[0]["bytes"], 99);
        assert_eq!(lines[1]["id"], Value::Null);
        assert_eq!(lines[1]["ok"], false);
        assert_eq!(lines[2]["id"], "2");
        assert_eq!(lines[2]["ok"], false);
        assert_eq!(lines[2]["error"], "input not found: missing.png");
        assert_eq!(lines[3]["id"], "3");
        assert_eq!(lines[3]["ok"], false);
        assert_eq!(lines[4]["id"], "4");
        assert_eq!(lines[4]["ok"], false);
        assert_eq!(lines[4]["error"], "inference failed: test");
        assert_eq!(lines[5]["id"], "5");
        assert_eq!(lines[5]["ok"], true);
    }

    #[test]
    fn the_next_image_is_read_and_prepared_while_earlier_ones_finish() {
        let (prepared, seen) = mpsc::channel();
        let input: String = (1..=3)
            .map(|n| {
                format!("{{\"id\":\"{n}\",\"input\":\"{n}.png\",\"output\":\"{n}-out.png\"}}\n")
            })
            .collect();
        let mut output = Vec::new();
        serve_lines(
            &mut Cursor::new(input),
            &mut output,
            &OutputOptions::default(),
            |request| {
                prepared.send(request.id.clone()).unwrap();
                Ok(request.id.clone())
            },
            Ok,
            move |id| {
                if id == "1" {
                    // Sequential serving reads request 3 only after answering 1.
                    let deadline = Instant::now() + Duration::from_secs(10);
                    let ahead = std::iter::from_fn(|| {
                        seen.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                            .ok()
                    })
                    .any(|id| id == "3");
                    assert!(ahead, "request 3 was not read while request 1 was written");
                }
                Ok((PathBuf::from(id), WRITTEN))
            },
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        let ids: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["id"].clone())
            .collect();
        assert_eq!(ids, ["1", "2", "3"]);
    }

    #[test]
    fn a_closed_stdout_ends_serving_with_its_error() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let input: String = (1..=20)
            .map(|n| {
                format!("{{\"id\":\"{n}\",\"input\":\"{n}.png\",\"output\":\"{n}-out.png\"}}\n")
            })
            .collect();
        let mut read = 0;
        let error = serve_lines(
            &mut Cursor::new(input),
            &mut Closed,
            &OutputOptions::default(),
            |request| {
                read += 1;
                Ok(request.output.clone())
            },
            Ok,
            |output| Ok((output, WRITTEN)),
        )
        .unwrap_err();
        assert!(error.message().contains("cannot write to stdout"));
        // Reading stops soon after the writer does instead of draining stdin.
        assert!(read < 20, "read {read} requests");
    }

    #[test]
    fn empty_input_ends_cleanly_without_output() {
        let (lines, processed) = served("");
        assert!(lines.is_empty() && processed.is_empty());
    }

    #[test]
    fn oversized_line_is_rejected_and_serving_continues() {
        let mut input = vec![b'x'; MAX_REQUEST_BYTES + 10];
        input.extend_from_slice(b"\n{\"id\":\"next\",\"input\":\"a.png\",\"output\":\"b.png\"}\n");
        let (lines, processed) = served(input);
        assert_eq!(processed, ["next"]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["id"], Value::Null);
        assert!(
            lines[0]["error"]
                .as_str()
                .unwrap()
                .contains("request line exceeds")
        );
        assert_eq!(lines[1]["id"], "next");
        assert_eq!(lines[1]["ok"], true);
    }

    #[test]
    fn unsafe_requests_are_rejected_before_inference() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path();
        let input = folder.join("photo.png");
        image::RgbaImage::new(2, 2).save(&input).unwrap();
        let model = folder.join("model.onnx");
        fs::write(&model, b"not a real model").unwrap();
        let half = folder.join("model-fp16.onnx");
        fs::write(&half, b"not a real model either").unwrap();
        let protected = [model.clone(), half.clone()];
        let check = |input: &Path, output: &Path| {
            check_request(
                &Request {
                    id: "1".into(),
                    input: input.to_path_buf(),
                    output: output.to_path_buf(),
                    options: OutputOptions::default(),
                },
                &protected,
            )
        };

        let output = folder.join("cut.png");
        check(&input, &output).unwrap();

        let gif = folder.join("misnamed-gif.png");
        image::RgbaImage::new(2, 2)
            .save_with_format(&gif, ImageFormat::Gif)
            .unwrap();
        let mut cases = vec![
            (
                folder.join("missing.png"),
                output.clone(),
                "input not found",
            ),
            (folder.to_path_buf(), output.clone(), "input is not a file"),
            (gif, output.clone(), "GIF"),
            (input.clone(), input.clone(), "overwrite the input"),
            (input.clone(), model.clone(), "overwrite the ONNX model"),
            (input.clone(), half.clone(), "overwrite the ONNX model"),
            (input.clone(), folder.to_path_buf(), "output is a directory"),
            (
                input.clone(),
                folder.join("new").join("cut.png"),
                "output folder does not exist",
            ),
        ];
        if cfg!(windows) {
            cases.push((
                input.clone(),
                folder.join("PHOTO.PNG"),
                "overwrite the input",
            ));
        }
        for (input, output, expected) in cases {
            let error = check(&input, &output).unwrap_err();
            assert!(error.message().contains(expected), "{}", error.message());
        }
        assert!(!folder.join("new").exists());
        assert!(!output.exists());
        assert_eq!(fs::read(&model).unwrap(), b"not a real model");
    }
}
