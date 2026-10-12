//! Resident `--serve` mode: load the model once, then answer one JSON request
//! per stdin line with exactly one JSON line on stdout, in request order.
//! Stdout is the protocol channel; human status goes to stderr only.
//!
//! Requests pass through three threads in order, so the model never waits for
//! image work: this thread reads and decodes the next image while the model
//! runs on the current one, and a writer thread saves the one before.

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use image::codecs::png::CompressionType;
use image::{ImageFormat, ImageReader};
use serde::Serialize;
use serde_json::Value;

use crate::commands::image_ops;
use crate::error::{ToolError, ToolResult};

use super::image_pipeline::PreparedImage;
use super::runtime::{self, Engine};
use super::{Provider, TOOL, jobs, model};

/// Longest accepted request line. Paths alone stay far below this; the cap
/// keeps a runaway caller from growing one line without bound.
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// Requests that may wait between two threads. Each holds its decoded image,
/// so the queues stay short; a full queue pauses reading until the model
/// catches up.
const QUEUED_REQUESTS: usize = 2;

pub(super) fn run(provider: Provider, download: bool, model_override: Option<&Path>) -> ToolResult {
    // Startup mirrors a normal run, including its download permission rules.
    let started = Instant::now();
    let runtime = runtime::initialize(provider, download)?;
    eprintln!("Runtime: {} ({})", runtime.path.display(), runtime.source);
    let model_path = model::resolve(model_override, download)?;
    eprintln!("Model: {}", model_path.display());
    let half = model::half_precision(model_override, &model_path, provider);
    let load_started = Instant::now();
    let (mut engine, attempts) = Engine::create(&model_path, half.as_deref(), provider, &runtime)?;
    for attempt in attempts {
        eprintln!("{TOOL}: {attempt}");
    }
    // The first inference prepares the GPU for this model and takes seconds;
    // running it now keeps that wait out of the first request.
    let size = engine.size();
    let blank = vec![0.0; 3 * size.pixels()];
    super::infer_with_fallback(&mut engine, &model_path, provider, &blank, report)?;
    eprintln!(
        "Model loaded in {:.1}s — selected provider: {}",
        load_started.elapsed().as_secs_f64(),
        super::provider_label(&engine)
    );

    let mut stdout = io::stdout();
    write_line(
        &mut stdout,
        &Ready {
            ready: true,
            provider: engine.provider().name(),
            load_seconds: seconds(started.elapsed()),
        },
    )?;
    eprintln!("{TOOL}: serving; send one JSON request per stdin line");
    serve_lines(
        &mut io::stdin().lock(),
        &mut stdout,
        |request| {
            let output = check_request(request, &model_path)?;
            Ok((PreparedImage::load(&request.input, size)?, output))
        },
        |(prepared, output)| {
            // The engine is shared, so a GPU-to-CPU fallback lasts for every
            // later request too.
            let mask = super::infer_with_fallback(
                &mut engine,
                &model_path,
                provider,
                &prepared.chw,
                report,
            )?;
            Ok((prepared, mask, output))
        },
        |(prepared, mask, output)| {
            // Callers of a resident model usually convert its cutouts, so
            // speed matters more than file size here.
            prepared.write_with_mask(&mask, &output, CompressionType::Fast)?;
            Ok(output)
        },
    )
}

#[derive(Serialize)]
struct Ready {
    ready: bool,
    provider: &'static str,
    load_seconds: f64,
}

#[derive(Debug, PartialEq)]
struct Request {
    id: String,
    input: PathBuf,
    output: PathBuf,
}

/// Why a line is not a usable request; `id` is echoed when it was readable.
#[derive(Debug, PartialEq)]
struct Rejected {
    id: Option<String>,
    message: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Response {
    Success {
        id: String,
        ok: bool,
        output: String,
        seconds: f64,
        stages: Stages,
    },
    Failure {
        id: Option<String>,
        ok: bool,
        error: String,
    },
}

/// Seconds a request spent in each step. Its `seconds` also counts the time it
/// waited behind earlier requests.
#[derive(Clone, Copy, Debug, Default, Serialize)]
struct Stages {
    prepare: f64,
    infer: f64,
    write: f64,
}

impl Response {
    fn success(id: String, output: &Path, elapsed: Duration, stages: Stages) -> Self {
        Self::Success {
            id,
            ok: true,
            output: output.to_string_lossy().into_owned(),
            seconds: seconds(elapsed),
            stages,
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
        stages: Stages,
        work: T,
    },
    Answered(Response),
}

/// Answers every non-blank line until end of input, in order. `prepare` runs
/// on this thread as each line is read, `infer` on a second thread and `write`
/// (which returns the absolute output path) on a third, so three requests can
/// be in progress at once.
fn serve_lines<R, W, P, Q>(
    input: &mut R,
    output: &mut W,
    mut prepare: impl FnMut(&Request) -> ToolResult<P>,
    infer: impl FnMut(P) -> ToolResult<Q> + Send,
    write: impl FnMut(Q) -> ToolResult<PathBuf> + Send,
) -> ToolResult
where
    R: BufRead,
    W: Write + Send,
    P: Send,
    Q: Send,
{
    thread::scope(|scope| {
        let (to_model, model_queue) = mpsc::sync_channel(QUEUED_REQUESTS);
        let (to_writer, writer_queue) = mpsc::sync_channel(QUEUED_REQUESTS);
        let model = scope.spawn(move || run_model(model_queue, to_writer, infer));
        let writer = scope.spawn(move || write_answers(writer_queue, output, write));
        let read = read_requests(input, &to_model, &mut prepare);
        drop(to_model);
        let stopped = |_| ToolError::new(TOOL, "a serve thread stopped unexpectedly");
        model.join().map_err(stopped)?;
        // A writer that cannot write (stdout closed) is why reading stopped early.
        writer.join().map_err(stopped)??;
        read
    })
}

fn read_requests<R: BufRead, P>(
    input: &mut R,
    next: &SyncSender<Step<P>>,
    prepare: &mut impl FnMut(&Request) -> ToolResult<P>,
) -> ToolResult {
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
            return Ok(());
        }
        let step = if line.len() > MAX_REQUEST_BYTES && line.last() != Some(&b'\n') {
            input.skip_until(b'\n').map_err(read_error)?;
            Step::Answered(Response::failure(
                None,
                format!("request line exceeds {MAX_REQUEST_BYTES} bytes"),
            ))
        } else {
            let text = line.trim_ascii();
            if text.is_empty() {
                continue;
            }
            match parse_request(text) {
                Ok(request) => {
                    let read = Instant::now();
                    match prepare(&request) {
                        Ok(work) => Step::Working {
                            id: request.id,
                            read,
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
            }
        };
        if next.send(step).is_err() {
            // The writer stopped and reports why.
            return Ok(());
        }
    }
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
                mut stages,
                work,
            } => {
                let started = Instant::now();
                match infer(work) {
                    Ok(work) => {
                        stages.infer = seconds(started.elapsed());
                        Step::Working {
                            id,
                            read,
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

fn write_answers<W: Write, Q>(
    queue: Receiver<Step<Q>>,
    output: &mut W,
    mut write: impl FnMut(Q) -> ToolResult<PathBuf>,
) -> ToolResult {
    for step in queue {
        let response = match step {
            Step::Working {
                id,
                read,
                mut stages,
                work,
            } => {
                let started = Instant::now();
                match write(work) {
                    Ok(path) => {
                        stages.write = seconds(started.elapsed());
                        Response::success(id, &path, read.elapsed(), stages)
                    }
                    Err(error) => Response::failure(Some(id), error.message()),
                }
            }
            Step::Answered(response) => response,
        };
        write_line(output, &response)?;
    }
    Ok(())
}

fn parse_request(line: &[u8]) -> Result<Request, Rejected> {
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
        id: id.to_owned(),
    })
}

/// Rejects requests that cannot or must not run, before any decoding, and
/// returns the absolute output path.
fn check_request(request: &Request, model: &Path) -> ToolResult<PathBuf> {
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
    jobs::reject_model_overwrite(std::slice::from_ref(&job), model)?;
    Ok(job.output)
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

fn seconds(elapsed: Duration) -> f64 {
    (elapsed.as_secs_f64() * 100.0).round() / 100.0
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

    fn served(input: impl AsRef<[u8]>) -> (Vec<Value>, Vec<String>) {
        let mut output = Vec::new();
        let mut processed = Vec::new();
        serve_lines(
            &mut Cursor::new(input),
            &mut output,
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
            |output| Ok(PathBuf::from("/abs").join(output)),
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
        )
        .unwrap();
        assert_eq!(
            request,
            Request {
                id: "7".into(),
                input: PathBuf::from(r"C:\frames\in.webp"),
                output: PathBuf::from("out.png"),
            }
        );
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
            let rejected = parse_request(line.as_bytes()).unwrap_err();
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
            let rejected = parse_request(line.as_bytes()).unwrap_err();
            assert_eq!(rejected.id.as_deref(), Some("a1"), "{line}");
            assert!(rejected.message.contains(field), "{}", rejected.message);
        }
    }

    #[test]
    fn ready_line_matches_the_protocol() {
        let ready = Ready {
            ready: true,
            provider: "DirectML",
            load_seconds: seconds(Duration::from_millis(9_104)),
        };
        assert_eq!(
            line_of(&ready),
            "{\"ready\":true,\"provider\":\"DirectML\",\"load_seconds\":9.1}\n"
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
                infer: 0.31,
                write: 0.02,
            },
        ));
        assert_eq!(
            success,
            r#"{"id":"x\"1","ok":true,"output":"C:\\cut \"outs\"\\frame-0.png","seconds":0.92,"stages":{"prepare":0.05,"infer":0.31,"write":0.02}}"#
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
                Ok(PathBuf::from(id))
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
            |request| {
                read += 1;
                Ok(request.output.clone())
            },
            Ok,
            Ok,
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
        let check = |input: &Path, output: &Path| {
            check_request(
                &Request {
                    id: "1".into(),
                    input: input.to_path_buf(),
                    output: output.to_path_buf(),
                },
                &model,
            )
        };

        let output = folder.join("cut.png");
        assert_eq!(
            check(&input, &output).unwrap(),
            std::path::absolute(&output).unwrap()
        );

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
