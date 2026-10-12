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

use image::codecs::png::CompressionType;

use crate::error::{ToolError, ToolResult};

use image_pipeline::PreparedImage;
use runtime::Engine;

const TOOL: &str = "justrmbg";
const HELP: &str = r#"justrmbg — Remove image backgrounds locally with BRIA RMBG-2.0.

Usage:
  justrmbg <image> [more images...] [options]
  justrmbg <inputDir> -o <outputDir>
  justrmbg --serve [--provider EP | --cpu | --gpu] [--model PATH] [--download]
  rmbg <image> [more images...] [options]

Options:
  -o, --output PATH  Output file for one image, or output directory in batch mode
      --provider EP  auto, cpu, directml, cuda, or coreml (default: auto)
      --cpu          Alias for --provider cpu
      --gpu          Require the platform-default GPU provider; never use CPU
      --check        Check runtime and acceleration without downloading the model
      --download     Install pinned managed runtime/model without a second prompt
      --model PATH   ONNX model path (or set RMBG_MODEL)
      --serve        Keep the model loaded and answer JSON requests from stdin
  -h, --help         Show this help

Default output is <input>-nobg.png beside the input. Inputs are always kept;
existing outputs are replaced atomically. For one input, --output is an exact
file; for multiple inputs or one directory, it is an output directory. Auto
mode uses managed DirectML on Windows x64 when available and may use CPU for
unsupported model nodes; if acceleration fails, it visibly falls back to CPU.
GPU runs use rmbg-2.0-fp16.onnx beside the managed model when it is there and
matches its pinned SHA-256 (see docs/rmbg.md).
Linux CUDA and macOS CoreML use a provider-enabled runtime supplied through
ORT_DYLIB_PATH.

The launcher enables --download visibly, so choosing Run installs a missing
managed runtime/model automatically. Headless runs must pass --download or
confirm interactively. Downloads use pinned sizes and SHA-256 hashes and are
cached per user.

--serve is for long-running callers. It loads the model once, prints
{"ready":true,"provider":"DirectML","load_seconds":9.1} on stdout, then reads
one JSON request per stdin line, {"id":"1","input":"in.jpg","output":"out.png"},
and answers each in order with one line: {"id":"1","ok":true,"output":
"<absolute path>","seconds":0.42,"stages":{...}} or {"id":"1","ok":false,
"error":"..."}. Send requests without waiting for answers: the next image is
decoded and the last one saved while the model runs. Its PNGs favor speed
over size. The output folder must exist and the input is never overwritten.
Stdout carries only these lines; status goes to stderr. End of input exits 0.

Run bare to open the interactive launcher. Its Headless footer shows the
equivalent direct command; explicit arguments bypass the UI."#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Provider {
    Auto,
    Cpu,
    DirectMl,
    Cuda,
    CoreMl,
}

impl Provider {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "cpu" => Some(Self::Cpu),
            "directml" | "dml" => Some(Self::DirectMl),
            "cuda" => Some(Self::Cuda),
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
    let value = if let Some(value) = inline {
        value
    } else {
        *index += 1;
        args.get(*index)
            .and_then(|value| value.to_str())
            .ok_or_else(|| ToolError::usage(TOOL, format!("{option} needs a value")))?
    };
    Provider::parse(value).ok_or_else(|| {
        ToolError::usage(
            TOOL,
            format!("unknown provider '{value}'; expected auto, cpu, directml, cuda, or coreml"),
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
    };
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
                "--check" => options.check = true,
                "--download" => options.download = true,
                "--serve" => options.serve = true,
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
    Ok(options)
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let options = parse(args)?;
    if options.help {
        println!("{HELP}");
        return Ok(());
    }

    if options.serve {
        return serve::run(options.provider, options.download, options.model.as_deref());
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
    let job_set = jobs::prepare(&options.inputs, options.output.as_deref())?;
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
    // Resolve the small native runtime before offering the ~780 MiB model download.
    let runtime = runtime::initialize(options.provider, options.download)?;
    status(&format!(
        "Runtime: {} ({})",
        runtime.path.display(),
        runtime.source
    ));
    let model_path = model::resolve(options.model.as_deref(), options.download)?;
    status(&format!("Model: {}", model_path.display()));
    jobs::reject_model_overwrite(&job_set.jobs, &model_path)?;
    let half = model::half_precision(options.model.as_deref(), &model_path, options.provider);
    if let Some(half) = &half {
        jobs::reject_model_overwrite(&job_set.jobs, half)?;
    }

    let load_started = Instant::now();
    let (mut engine, attempts) =
        Engine::create(&model_path, half.as_deref(), options.provider, &runtime)?;
    for attempt in attempts {
        eprintln!("{TOOL}: {attempt}");
    }
    status(&format!(
        "Model loaded in {:.1}s — selected provider: {}",
        load_started.elapsed().as_secs_f64(),
        provider_label(&engine)
    ));

    let piped = !io::stdout().is_terminal();
    let mut failures = 0_usize;
    for job in &job_set.jobs {
        let started = Instant::now();
        if let Err(error) = process_job(
            &mut engine,
            &model_path,
            options.provider,
            &job.input,
            &job.output,
            status,
        ) {
            if !job_set.batch_mode {
                return Err(error);
            }
            failures += 1;
            eprintln!("{TOOL}: {}: {}", job.input.display(), error.message());
            continue;
        }

        status(&format!(
            "{} -> {} ({:.1}s)",
            job.input.display(),
            job.output.display(),
            started.elapsed().as_secs_f64()
        ));
        if piped {
            let absolute = std::path::absolute(&job.output).unwrap_or_else(|_| job.output.clone());
            println!("{}", absolute.display());
        }
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

/// Removes one image's background.
fn process_job(
    engine: &mut Engine,
    model_path: &Path,
    provider: Provider,
    input: &Path,
    output: &Path,
    report: fn(&str),
) -> ToolResult {
    // Decode first so corrupt inputs never trigger GPU-to-CPU fallback.
    let prepared = PreparedImage::load(input, engine.size())?;
    let mask = infer_with_fallback(engine, model_path, provider, &prepared.chw, report)?;
    prepared.write_with_mask(&mask, output, CompressionType::Default)
}

/// Runs the model on one prepared image. When Auto's GPU inference fails, the
/// engine moves to the CPU (with the full model) for this and every later image.
/// `report` receives human status lines, so serve mode can keep them off its
/// stdout protocol channel.
fn infer_with_fallback(
    engine: &mut Engine,
    model_path: &Path,
    provider: Provider,
    chw: &[f32],
    report: fn(&str),
) -> ToolResult<Vec<f32>> {
    match engine.infer(chw) {
        Ok(mask) => Ok(mask),
        Err(gpu_error) if provider == Provider::Auto && engine.is_gpu() => {
            eprintln!(
                "{TOOL}: {} inference failed — falling back to CPU.\n{}",
                engine.provider().name(),
                gpu_error.message()
            );
            engine.replace_with_cpu(model_path)?;
            report("Provider changed to CPU for this and remaining images");
            engine.infer(chw)
        }
        Err(error) => Err(error),
    }
}

/// The provider's name, noting the half-precision model.
fn provider_label(engine: &Engine) -> String {
    if engine.is_half_precision() {
        format!("{} (half precision)", engine.provider().name())
    } else {
        engine.provider().name().to_owned()
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
