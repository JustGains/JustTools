mod host;
mod image_pipeline;
mod jobs;
mod model;
mod runtime;
mod serve;

use std::ffi::OsString;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::{ToolError, ToolResult};

use image_pipeline::{Format, ModelSize, OutputOptions, PngCompression, PreparedImage};
use model::ModelSet;
use runtime::Engine;

const TOOL: &str = "justrmbg";
const HELP: &str = r#"justrmbg — Remove image backgrounds locally with BRIA RMBG-2.0.

Usage:
  justrmbg <image> [more images...] [options]
  justrmbg <inputDir> -o <outputDir>
  justrmbg --serve [--provider EP | --cpu | --gpu] [--model PATH] [--download] [output options]
  rmbg <image> [more images...] [options]

Options:
  -o, --output PATH     Output file for one image, or output directory in batch mode
      --provider EP     auto, cpu, directml, cuda, tensorrt, or coreml (default: auto)
      --cpu             Alias for --provider cpu
      --gpu             Require the platform-default GPU provider; never use CPU
      --full-precision  Keep float32 on GPUs: masks exactly as BRIA's reference, about twice as slow
      --check           Check runtime and acceleration without downloading the model
      --download        Install pinned managed runtime/model without a second prompt
      --model PATH      ONNX model path (or set RMBG_MODEL)
      --serve           Keep the model loaded and answer JSON requests from stdin
  -h, --help            Show this help

Output options (serve requests take the same, in camelCase):
      --format png|webp       File format (default: the output's extension, else png)
      --crop                  Crop to the visible subject
      --padding PX            Transparent room around the subject (default 0)
      --max-width PX          Scale down (never up) to fit
      --max-height PX         Scale down (never up) to fit
      --quality N             WebP quality 0-100 (default 80)
      --alpha-quality N       WebP alpha quality 0-100 (default 80)
      --method N              WebP effort 0-6 (default 4)
      --compression fast|small  PNG compression (default fast)
      --picture               A finished picture: --crop --padding 10
                              --max-height 400 --format webp

Default output is <input>-nobg.png (or .webp) beside the input. Inputs are
always kept; existing outputs are replaced atomically. For one input, --output
is an exact file; for multiple inputs or one directory, it is an output
directory. Auto mode uses managed DirectML on Windows x64 when available and
may use CPU for unsupported model nodes; if acceleration fails, it visibly
falls back to CPU. Each provider runs the best verified model file beside the
managed model (docs/rmbg.md, "Model files"): the fast half-precision export on
GPUs, the low-memory export on CPUs. TensorRT, CUDA (Windows and Linux) and
CoreML use a provider-enabled runtime supplied through ORT_DYLIB_PATH.

The launcher enables --download visibly, so choosing Run installs a missing
managed runtime/model automatically. Headless runs must pass --download or
confirm interactively. Downloads use pinned sizes and SHA-256 hashes and are
cached per user.

--serve is for long-running callers. It loads the model once, prints
{"ready":true,"provider":"DirectML","model":"rmbg-2.0-fast-fp16",
"load_seconds":2.3,"pictures":true} on stdout, then reads one JSON request per
stdin line, {"id":"1","input":"in.jpg","output":"out.png"} plus any output
options ({"picture":true} or {"picture":{"maxHeight":300}} for a finished
picture), and answers each in order with one line: {"id":"1","ok":true,
"output":"<absolute path>","seconds":0.12,"stages":{...},"width":...,
"height":...,"bytes":...} or {"id":"1","ok":false,"error":"..."}. Output
options given with --serve are every request's defaults. Send requests without
waiting for answers: the next image is decoded and the last one saved while
the model runs; command-line batches run the same way. The output folder must
exist and the input is never overwritten. Stdout carries only these lines;
status goes to stderr. End of input exits 0.

Run bare to open the interactive launcher. Its Headless footer shows the
equivalent direct command; explicit arguments bypass the UI."#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Provider {
    Auto,
    Cpu,
    DirectMl,
    Cuda,
    TensorRt,
    CoreMl,
}

impl Provider {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "cpu" => Some(Self::Cpu),
            "directml" | "dml" => Some(Self::DirectMl),
            "cuda" => Some(Self::Cuda),
            "tensorrt" | "trt" => Some(Self::TensorRt),
            "coreml" => Some(Self::CoreMl),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Cpu => "CPU",
            Self::DirectMl => "DirectML",
            Self::Cuda => "CUDA",
            Self::TensorRt => "TensorRT",
            Self::CoreMl => "CoreML",
        }
    }

    fn platform_gpu() -> Self {
        if cfg!(target_os = "windows") {
            Self::DirectMl
        } else if cfg!(target_os = "macos") {
            Self::CoreMl
        } else {
            Self::Cuda
        }
    }
}

#[derive(Debug)]
struct Options {
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    provider: Provider,
    provider_explicit: bool,
    check: bool,
    download: bool,
    model: Option<PathBuf>,
    serve: bool,
    help: bool,
    full_precision: bool,
    /// How cut-outs are saved; with --serve, every request's defaults.
    output_options: OutputOptions,
    /// Whether any output option was given (they are no use with --check).
    output_options_given: bool,
}

fn option_path(
    args: &[OsString],
    index: &mut usize,
    option: &str,
    inline: Option<&str>,
) -> ToolResult<PathBuf> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(ToolError::usage(TOOL, format!("{option} needs a value")));
        }
        return Ok(PathBuf::from(value));
    }
    *index += 1;
    args.get(*index)
        .map(PathBuf::from)
        .ok_or_else(|| ToolError::usage(TOOL, format!("{option} needs a value")))
}

fn option_text<'a>(
    args: &'a [OsString],
    index: &mut usize,
    option: &str,
    inline: Option<&'a str>,
) -> ToolResult<&'a str> {
    if let Some(value) = inline {
        return Ok(value);
    }
    *index += 1;
    args.get(*index)
        .and_then(|value| value.to_str())
        .ok_or_else(|| ToolError::usage(TOOL, format!("{option} needs a value")))
}

fn option_number(
    args: &[OsString],
    index: &mut usize,
    option: &str,
    inline: Option<&str>,
    low: u32,
    high: u32,
) -> ToolResult<u32> {
    let text = option_text(args, index, option, inline)?;
    text.parse::<u32>()
        .ok()
        .filter(|value| (low..=high).contains(value))
        .ok_or_else(|| {
            ToolError::usage(
                TOOL,
                format!("{option} must be a whole number from {low} to {high}, not '{text}'"),
            )
        })
}

fn set_provider(options: &mut Options, requested: Provider, option: &str) -> ToolResult {
    if options.provider_explicit && options.provider != requested {
        return Err(ToolError::usage(
            TOOL,
            format!(
                "conflicting provider options: {} and {option}",
                options.provider.name()
            ),
        ));
    }
    options.provider = requested;
    options.provider_explicit = true;
    Ok(())
}

fn provider_value(
    args: &[OsString],
    index: &mut usize,
    option: &str,
    inline: Option<&str>,
) -> ToolResult<Provider> {
    let value = option_text(args, index, option, inline)?;
    Provider::parse(value).ok_or_else(|| {
        ToolError::usage(
            TOOL,
            format!(
                "unknown provider '{value}'; expected auto, cpu, directml, cuda, tensorrt, or coreml"
            ),
        )
    })
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        inputs: Vec::new(),
        output: None,
        provider: Provider::Auto,
        provider_explicit: false,
        check: false,
        download: false,
        model: None,
        serve: false,
        help: false,
        full_precision: false,
        output_options: OutputOptions::default(),
        output_options_given: false,
    };
    // --picture sets several options at once; options given beside it win
    // wherever they appear.
    let mut picture = false;
    let mut explicit = OutputOptions::default();
    let mut given = Vec::new();
    let mut positional = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        let text = argument.to_str();
        if !positional && text == Some("--") {
            positional = true;
            index += 1;
            continue;
        }

        if !positional {
            let (option, inline) = text
                .and_then(|value| {
                    value
                        .split_once('=')
                        .filter(|(name, _)| name.starts_with("--"))
                })
                .map_or((text.unwrap_or(""), None), |(name, value)| {
                    (name, Some(value))
                });
            match option {
                "-h" | "--help" => options.help = true,
                "-o" | "--output" => {
                    options.output = Some(option_path(&args, &mut index, option, inline)?);
                }
                "--model" => {
                    options.model = Some(option_path(&args, &mut index, option, inline)?);
                }
                "--provider" => {
                    let provider = provider_value(&args, &mut index, option, inline)?;
                    set_provider(&mut options, provider, option)?;
                }
                "--cpu" => set_provider(&mut options, Provider::Cpu, option)?,
                "--gpu" => set_provider(&mut options, Provider::platform_gpu(), option)?,
                "--full-precision" => options.full_precision = true,
                "--check" => options.check = true,
                "--download" => options.download = true,
                "--serve" => options.serve = true,
                "--picture" => {
                    picture = true;
                    given.push("picture");
                }
                "--format" => {
                    let value = option_text(&args, &mut index, option, inline)?;
                    explicit.format = Some(Format::parse(value).ok_or_else(|| {
                        ToolError::usage(
                            TOOL,
                            format!("--format must be png or webp, not '{value}'"),
                        )
                    })?);
                    given.push("format");
                }
                "--crop" => {
                    explicit.crop = true;
                    given.push("crop");
                }
                "--padding" => {
                    explicit.padding = option_number(&args, &mut index, option, inline, 0, 10_000)?;
                    given.push("padding");
                }
                "--max-width" => {
                    explicit.max_width = Some(option_number(
                        &args, &mut index, option, inline, 1, 100_000,
                    )?);
                    given.push("max-width");
                }
                "--max-height" => {
                    explicit.max_height = Some(option_number(
                        &args, &mut index, option, inline, 1, 100_000,
                    )?);
                    given.push("max-height");
                }
                "--quality" => {
                    explicit.quality =
                        option_number(&args, &mut index, option, inline, 0, 100)? as u8;
                    given.push("quality");
                }
                "--alpha-quality" => {
                    explicit.alpha_quality =
                        option_number(&args, &mut index, option, inline, 0, 100)? as u8;
                    given.push("alpha-quality");
                }
                "--method" => {
                    explicit.method = option_number(&args, &mut index, option, inline, 0, 6)? as u8;
                    given.push("method");
                }
                "--compression" => {
                    let value = option_text(&args, &mut index, option, inline)?;
                    explicit.png = match value {
                        "fast" => PngCompression::Fast,
                        "small" => PngCompression::Small,
                        _ => {
                            return Err(ToolError::usage(
                                TOOL,
                                format!("--compression must be fast or small, not '{value}'"),
                            ));
                        }
                    };
                    given.push("compression");
                }
                _ if text.is_some_and(|value| value.starts_with('-')) => {
                    return Err(ToolError::usage(
                        TOOL,
                        format!("unknown option: {}", argument.to_string_lossy()),
                    ));
                }
                _ => options.inputs.push(PathBuf::from(argument)),
            }
        } else {
            options.inputs.push(PathBuf::from(argument));
        }
        index += 1;
    }
    options.output_options = merge_output_options(picture, &explicit, &given);
    options.output_options_given = !given.is_empty();

    if options.serve {
        // Serve mode takes its images from stdin requests, one at a time.
        let conflict = if options.check {
            Some("--check")
        } else if options.output.is_some() {
            Some("--output")
        } else if !options.inputs.is_empty() {
            Some("image inputs")
        } else {
            None
        };
        if let Some(conflict) = conflict {
            return Err(ToolError::usage(
                TOOL,
                format!("--serve cannot be combined with {conflict}"),
            ));
        }
    }
    if !options.help && !options.check && !options.serve && options.inputs.is_empty() {
        return Err(ToolError::usage(
            TOOL,
            "at least one image or folder is required (or use --check)",
        ));
    }
    if options.check && !options.inputs.is_empty() {
        return Err(ToolError::usage(
            TOOL,
            "--check does not accept image inputs",
        ));
    }
    if options.check && (options.output.is_some() || options.model.is_some()) {
        return Err(ToolError::usage(
            TOOL,
            "--check cannot be combined with --output or --model",
        ));
    }
    if options.check && options.output_options_given {
        return Err(ToolError::usage(
            TOOL,
            "--check cannot be combined with output options",
        ));
    }
    Ok(options)
}

/// The output options a command line asks for: --picture's settings (if given)
/// under every option named explicitly.
fn merge_output_options(picture: bool, explicit: &OutputOptions, given: &[&str]) -> OutputOptions {
    let mut options = if picture {
        OutputOptions::picture()
    } else {
        OutputOptions::default()
    };
    for name in given {
        match *name {
            "format" => options.format = explicit.format,
            "crop" => options.crop = explicit.crop,
            "padding" => options.padding = explicit.padding,
            "max-width" => options.max_width = explicit.max_width,
            "max-height" => options.max_height = explicit.max_height,
            "quality" => options.quality = explicit.quality,
            "alpha-quality" => options.alpha_quality = explicit.alpha_quality,
            "method" => options.method = explicit.method,
            "compression" => options.png = explicit.png,
            _ => {}
        }
    }
    options
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let options = parse(args)?;
    if options.help {
        println!("{HELP}");
        return Ok(());
    }

    if options.serve {
        return serve::run(
            options.provider,
            options.download,
            options.model.as_deref(),
            options.full_precision,
            &options.output_options,
        );
    }

    if options.check {
        let runtime = runtime::initialize(options.provider, options.download)?;
        status(&format!(
            "Runtime: {} ({})",
            runtime.path.display(),
            runtime.source
        ));
        return runtime::check(options.provider, &runtime);
    }

    // Validate all cheap file mappings before offering any dependency download.
    let extension = options
        .output_options
        .format
        .unwrap_or(Format::Png)
        .extension();
    let job_set = jobs::prepare(&options.inputs, options.output.as_deref(), extension)?;
    for job in &job_set.jobs {
        super::image_ops::assert_static(TOOL, &job.input)?;
        status(&format!(
            "Output: {} -> {}{}",
            job.input.display(),
            job.output.display(),
            if job.output.exists() {
                " (existing output will be replaced atomically)"
            } else {
                " (input will be kept)"
            }
        ));
    }
    let started = Instant::now();
    let mut session = Session::start(
        options.provider,
        options.download,
        options.model.as_deref(),
        options.full_precision,
        status,
    )?;
    for model in session.protected_paths() {
        jobs::reject_model_overwrite(&job_set.jobs, &model)?;
    }
    status(&format!(
        "Model loaded in {:.1}s — selected provider: {}",
        started.elapsed().as_secs_f64(),
        session.label()
    ));

    // The same three stages as serve mode: the next image is decoded and the
    // last one saved while the model runs.
    let piped = !io::stdout().is_terminal();
    let size = session.size();
    let (engine, fallback) = session.parts();
    let requests = job_set.jobs.iter().enumerate().map(|(index, job)| {
        Ok(serve::Request {
            id: index.to_string(),
            input: job.input.clone(),
            output: job.output.clone(),
            options: options.output_options.clone(),
        })
    });
    let mut failures = 0_usize;
    let mut single_error = None;
    serve::run_requests(
        requests,
        |response| {
            match response {
                serve::Response::Success {
                    id,
                    output,
                    seconds,
                    ..
                } => {
                    let job = &job_set.jobs[id.parse::<usize>().unwrap_or(0)];
                    status(&format!(
                        "{} -> {} ({seconds:.2}s)",
                        job.input.display(),
                        job.output.display()
                    ));
                    if piped {
                        println!("{output}");
                    }
                }
                serve::Response::Failure { id, error, .. } => {
                    failures += 1;
                    let job = id
                        .and_then(|id| id.parse::<usize>().ok())
                        .map(|index| &job_set.jobs[index]);
                    if job_set.batch_mode {
                        let input = job.map(|job| job.input.display().to_string());
                        eprintln!("{TOOL}: {}: {error}", input.unwrap_or_default());
                    } else {
                        single_error = Some(error);
                    }
                }
            }
            Ok(())
        },
        |request| {
            PreparedImage::load(&request.input, size).map(|prepared| {
                (
                    prepared,
                    serve::absolute(&request.output),
                    request.options.clone(),
                )
            })
        },
        |(prepared, output, output_options)| {
            // Decoded first, so corrupt inputs never trigger GPU-to-CPU fallback.
            let mask = infer_with_fallback(engine, fallback, &prepared.chw, status)?;
            Ok((prepared, mask, output, output_options))
        },
        |(prepared, mask, output, output_options)| {
            let written = prepared.write_with_mask(&mask, &output, &output_options)?;
            Ok((output, written))
        },
    )?;
    if let Some(error) = single_error {
        return Err(ToolError::new(TOOL, error));
    }

    let succeeded = job_set.jobs.len() - failures;
    if job_set.batch_mode {
        status(&format!(
            "Completed: {succeeded} succeeded, {failures} failed"
        ));
    }
    if failures > 0 {
        return Err(ToolError::new(
            TOOL,
            format!("batch incomplete: {succeeded} succeeded, {failures} failed"),
        ));
    }
    Ok(())
}

/// What a run needs before its first image: the runtime, the model files and
/// the loaded engine. Command-line runs and serve mode start the same way.
pub(crate) struct Session {
    engine: Engine,
    fallback: Fallback,
}

/// Where Auto goes when its GPU fails during a run: the CPU, with the CPU's
/// best model file.
pub(crate) struct Fallback {
    provider: Provider,
    models: ModelSet,
}

impl Session {
    fn start(
        provider: Provider,
        download: bool,
        model_override: Option<&Path>,
        full_precision: bool,
        report: fn(&str),
    ) -> ToolResult<Self> {
        // Resolve the small native runtime before offering the ~780 MiB model download.
        let runtime = runtime::initialize(provider, download)?;
        report(&format!(
            "Runtime: {} ({})",
            runtime.path.display(),
            runtime.source
        ));
        let model_path = model::resolve(model_override, download)?;
        report(&format!("Model: {}", model_path.display()));
        let models = ModelSet::new(&model_path, model_override, full_precision);
        let (engine, attempts) = Engine::create(&models, provider, &runtime)?;
        for attempt in attempts {
            eprintln!("{TOOL}: {attempt}");
        }
        Ok(Self {
            engine,
            fallback: Fallback { provider, models },
        })
    }

    /// One inference on a blank picture: the first one prepares the GPU for the
    /// model and takes seconds, which a long-running caller should not wait for.
    fn warm_up(&mut self, report: fn(&str)) -> ToolResult {
        let blank = vec![0.0; 3 * self.engine.size().pixels()];
        infer_with_fallback(&mut self.engine, &self.fallback, &blank, report).map(|_| ())
    }

    /// The provider and the model file it runs.
    fn label(&self) -> String {
        format!(
            "{} ({})",
            self.engine.provider().name(),
            self.engine.model_name()
        )
    }

    fn provider_name(&self) -> &'static str {
        self.engine.provider().name()
    }

    fn model_name(&self) -> String {
        self.engine.model_name()
    }

    fn protected_paths(&self) -> Vec<PathBuf> {
        self.fallback.models.protected()
    }

    fn size(&self) -> ModelSize {
        self.engine.size()
    }

    fn parts(&mut self) -> (&mut Engine, &Fallback) {
        (&mut self.engine, &self.fallback)
    }
}

/// Runs the model on one prepared image. When Auto's GPU inference fails, the
/// engine moves to the CPU (with the CPU's model) for this and every later
/// image. `report` receives human status lines, so serve mode can keep them off
/// its stdout protocol channel.
fn infer_with_fallback(
    engine: &mut Engine,
    fallback: &Fallback,
    chw: &[f32],
    report: fn(&str),
) -> ToolResult<Vec<f32>> {
    match engine.infer(chw) {
        Ok(mask) => Ok(mask),
        Err(gpu_error) if fallback.provider == Provider::Auto && engine.is_gpu() => {
            eprintln!(
                "{TOOL}: {} inference failed — falling back to CPU.\n{}",
                engine.provider().name(),
                gpu_error.message()
            );
            engine.replace_with_cpu(&fallback.models)?;
            report("Provider changed to CPU for this and remaining images");
            engine.infer(chw)
        }
        Err(error) => Err(error),
    }
}

fn status(message: &str) {
    if io::stdout().is_terminal() {
        println!("{message}");
    } else {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gpu_alias_as_strict_platform_provider() {
        let options = parse(vec!["photo.jpg".into(), "--gpu".into()]).unwrap();
        assert_eq!(options.inputs, [PathBuf::from("photo.jpg")]);
        assert_eq!(options.provider, Provider::platform_gpu());
    }

    #[test]
    fn parses_all_named_providers() {
        for (value, expected) in [
            ("auto", Provider::Auto),
            ("cpu", Provider::Cpu),
            ("directml", Provider::DirectMl),
            ("cuda", Provider::Cuda),
            ("tensorrt", Provider::TensorRt),
            ("trt", Provider::TensorRt),
            ("coreml", Provider::CoreMl),
        ] {
            let options =
                parse(vec!["photo.jpg".into(), "--provider".into(), value.into()]).unwrap();
            assert_eq!(options.provider, expected);
        }
    }

    #[test]
    fn rejects_conflicting_providers() {
        let error = parse(vec!["x.png".into(), "--cpu".into(), "--gpu".into()]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn check_needs_no_input() {
        let options = parse(vec!["--check".into()]).unwrap();
        assert!(options.check);
        assert!(options.inputs.is_empty());
    }

    #[test]
    fn download_is_an_explicit_headless_permission() {
        let options = parse(vec!["photo.jpg".into(), "--download".into()]).unwrap();
        assert!(options.download);
    }

    #[test]
    fn check_rejects_image_arguments() {
        let error = parse(vec!["--check".into(), "x.png".into()]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn invalid_provider_is_usage_error() {
        let error = parse(vec!["x.png".into(), "--provider".into(), "vulkan".into()]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.message().contains("unknown provider"));
    }

    #[test]
    fn help_needs_no_input() {
        assert!(parse(vec!["--help".into()]).unwrap().help);
    }

    #[test]
    fn serve_needs_no_input_and_accepts_runtime_options() {
        let options = parse(vec!["--serve".into()]).unwrap();
        assert!(options.serve);
        assert!(options.inputs.is_empty());
        assert_eq!(options.provider, Provider::Auto);

        let options = parse(vec![
            "--serve".into(),
            "--provider".into(),
            "directml".into(),
            "--model".into(),
            "custom.onnx".into(),
            "--download".into(),
        ])
        .unwrap();
        assert!(options.serve && options.download);
        assert_eq!(options.provider, Provider::DirectMl);
        assert_eq!(options.model, Some(PathBuf::from("custom.onnx")));

        for alias in ["--cpu", "--gpu", "--provider=cpu", "--model=m.onnx"] {
            assert!(parse(vec!["--serve".into(), alias.into()]).unwrap().serve);
        }
    }

    #[test]
    fn output_options_parse_with_picture_as_a_base() {
        let options = parse(
            [
                "photo.jpg",
                "--picture",
                "--max-height=300",
                "--padding",
                "4",
            ]
            .iter()
            .map(OsString::from)
            .collect(),
        )
        .unwrap();
        assert_eq!(
            options.output_options,
            OutputOptions {
                max_height: Some(300),
                padding: 4,
                ..OutputOptions::picture()
            }
        );
        // an explicit option wins even when it comes before --picture
        let options = parse(
            ["photo.jpg", "--format", "png", "--picture"]
                .iter()
                .map(OsString::from)
                .collect(),
        )
        .unwrap();
        assert_eq!(options.output_options.format, Some(Format::Png));
        assert!(options.output_options.crop);

        let options = parse(
            [
                "photo.jpg",
                "--crop",
                "--max-width",
                "64",
                "--quality=90",
                "--alpha-quality",
                "100",
                "--method=6",
                "--compression",
                "small",
                "--full-precision",
            ]
            .iter()
            .map(OsString::from)
            .collect(),
        )
        .unwrap();
        assert!(options.full_precision);
        assert_eq!(
            options.output_options,
            OutputOptions {
                crop: true,
                max_width: Some(64),
                quality: 90,
                alpha_quality: 100,
                method: 6,
                png: PngCompression::Small,
                ..OutputOptions::default()
            }
        );
        assert_eq!(
            parse(vec!["photo.jpg".into()]).unwrap().output_options,
            OutputOptions::default()
        );
    }

    #[test]
    fn bad_output_options_are_usage_errors() {
        for args in [
            vec!["x.png", "--format", "gif"],
            vec!["x.png", "--quality", "101"],
            vec!["x.png", "--max-height", "0"],
            vec!["x.png", "--method=7"],
            vec!["x.png", "--compression", "zip"],
            vec!["x.png", "--padding", "-1"],
            vec!["--check", "--picture"],
        ] {
            let error = parse(args.iter().map(OsString::from).collect()).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
        // serve mode takes output options as its requests' defaults
        let options = parse(vec!["--serve".into(), "--picture".into()]).unwrap();
        assert!(options.serve);
        assert_eq!(options.output_options, OutputOptions::picture());
    }

    #[test]
    fn serve_rejects_inputs_output_and_check() {
        for args in [
            vec!["--serve", "photo.jpg"],
            vec!["photo.jpg", "--serve"],
            vec!["--serve", "--", "photo.jpg"],
            vec!["--serve", "-o", "out.png"],
            vec!["--serve", "--output=out.png"],
            vec!["--serve", "--check"],
        ] {
            let error = parse(args.iter().map(OsString::from).collect()).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{args:?}");
            assert!(error.message().contains("--serve"), "{args:?}");
        }
    }
}
