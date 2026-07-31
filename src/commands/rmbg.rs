mod image_pipeline;
mod jobs;
mod model;
mod runtime;

use std::ffi::OsString;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::{ToolError, ToolResult};

use image_pipeline::PreparedImage;
use runtime::Engine;

const TOOL: &str = "justrmbg";
const HELP: &str = r#"justrmbg — Remove image backgrounds locally with BRIA RMBG-2.0.

Usage:
  justrmbg <image> [more images...] [options]
  justrmbg <inputDir> -o <outputDir>
  rmbg <image> [more images...] [options]

Options:
  -o, --output PATH  Output file for one image, or output directory in batch mode
      --cpu          Force CPU inference
      --gpu          Force GPU inference; fail if no GPU provider initializes
      --model PATH   ONNX model path (or set RMBG_MODEL)
  -h, --help         Show this help

Default output is <input>-nobg.png. Automatic mode tries DirectML/CUDA on
Windows, CUDA on Linux, or CoreML on macOS before falling back to CPU.

The model and a portable CPU runtime are downloaded only after interactive
confirmation, verified with pinned SHA-256 hashes, and cached per user.
Noninteractive runs never download dependencies."#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Device {
    Auto,
    Cpu,
    Gpu,
}

#[derive(Debug)]
struct Options {
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    device: Device,
    model: Option<PathBuf>,
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

fn set_device(current: &mut Device, requested: Device) -> ToolResult {
    if *current != Device::Auto && *current != requested {
        return Err(ToolError::usage(
            TOOL,
            "--cpu and --gpu cannot be used together",
        ));
    }
    *current = requested;
    Ok(())
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        inputs: Vec::new(),
        output: None,
        device: Device::Auto,
        model: None,
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
                "--cpu" => set_device(&mut options.device, Device::Cpu)?,
                "--gpu" => set_device(&mut options.device, Device::Gpu)?,
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

    if !options.help && options.inputs.is_empty() {
        return Err(ToolError::usage(
            TOOL,
            "at least one image or folder is required",
        ));
    }
    if options.output.is_some() && options.inputs.len() > 1 {
        return Err(ToolError::usage(
            TOOL,
            "-o/--output can only be used with a single input image",
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

    let job_set = jobs::prepare(&options.inputs, options.output.as_deref())?;
    // Resolve the small native runtime before offering the ~780 MiB model download.
    runtime::initialize()?;
    let model_path = model::resolve(options.model.as_deref())?;
    jobs::reject_model_overwrite(&job_set.jobs, &model_path)?;

    let load_started = Instant::now();
    let mut engine = Engine::create(&model_path, options.device)?;
    let detected = match options.device {
        Device::Auto if engine.is_gpu() => {
            format!("GPU detected ({})", engine.provider().to_ascii_uppercase())
        }
        Device::Auto => "no GPU detected — using CPU".to_owned(),
        _ => engine.provider().to_ascii_uppercase(),
    };
    status(&format!(
        "Model loaded in {:.1}s — {detected}",
        load_started.elapsed().as_secs_f64()
    ));

    let piped = !io::stdout().is_terminal();
    let mut failures = 0_usize;
    for job in &job_set.jobs {
        let started = Instant::now();
        if let Err(error) = process_job(
            &mut engine,
            &model_path,
            options.device,
            &job.input,
            &job.output,
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

    if failures == job_set.jobs.len() {
        return Err(ToolError::new(
            TOOL,
            format!("all {failures} input file(s) failed"),
        ));
    }
    Ok(())
}

fn process_job(
    engine: &mut Engine,
    model_path: &Path,
    device: Device,
    input: &Path,
    output: &Path,
) -> ToolResult {
    // Decode first so corrupt inputs never trigger GPU-to-CPU fallback.
    let prepared = PreparedImage::load(input)?;
    let mask = match engine.infer(&prepared.chw) {
        Ok(mask) => mask,
        Err(gpu_error) if device == Device::Auto && engine.is_gpu() => {
            eprintln!(
                "{TOOL}: GPU ({}) inference failed — falling back to CPU.\n{}",
                engine.provider().to_ascii_uppercase(),
                gpu_error.message()
            );
            *engine = Engine::cpu(model_path)?;
            engine.infer(&prepared.chw)?
        }
        Err(error) => return Err(error),
    };
    prepared.write_with_mask(&mask, output)
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
    fn parses_original_contract() {
        let options = parse(vec!["photo.jpg".into(), "--gpu".into()]).unwrap();
        assert_eq!(options.inputs, [PathBuf::from("photo.jpg")]);
        assert_eq!(options.device, Device::Gpu);
    }

    #[test]
    fn rejects_conflicting_devices() {
        let error = parse(vec!["x.png".into(), "--cpu".into(), "--gpu".into()]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn help_needs_no_input() {
        assert!(parse(vec!["--help".into()]).unwrap().help);
    }
}
