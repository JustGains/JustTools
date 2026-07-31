use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(test)]
use image::DynamicImage;
use image::imageops::FilterType;
use rayon::prelude::*;

use crate::common::{self, CollectedPaths, InputOptions, Plan};
use crate::error::{ToolError, ToolResult};

use super::image_ops;

const TOOL: &str = "justresize";
const DEFAULT_MAX: u32 = 1920;
const DEFAULT_JPEG_QUALITY: u32 = 85;
const MAX_OUTPUT_PIXELS: u64 = 100_000_000;

#[derive(Clone, Debug)]
struct Options {
    width: Option<u32>,
    height: Option<u32>,
    max: Option<u32>,
    crop: bool,
    upscale: bool,
    output: Option<PathBuf>,
    replace: bool,
    recursive: bool,
    jobs: usize,
    quality: u32,
    yes: bool,
    dry_run: bool,
    inputs: Vec<OsString>,
    help: bool,
}

fn help() -> &'static str {
    r#"justresize — Resize still images quickly without changing their format.

Usage:
  justresize [options] [file-or-folder ...]

Default: fit within 1920x1920, preserve aspect ratio, never upscale, use
Lanczos3, keep the source, and write <name>-resized.<ext>. Images already
within the requested bounds are left unchanged.

Options:
  -w, --width PX         Target width; height follows the aspect ratio
  -H, --height PX        Target height; width follows the aspect ratio
  -m, --max PX           Fit within a square (default: 1920)
      --crop             Center-crop to exact --width and --height dimensions
      --upscale          Permit enlargement (off by default)
  -o, --output DIR       Write copies to DIR using the original names
      --replace          Atomically replace source images
  -q, --quality N        JPEG quality, 1-100 (default: 85)
  -j, --jobs N           Parallel resizes (default: up to 8)
  -r, --recursive        Include nested folders
  -y, --yes              Skip folder replacement confirmation
  -n, --dry-run          Show dimensions and outputs without writing
  -h, --help             Show this help

Supported still formats: JPEG, PNG, WebP, BMP, TIFF, and QOI. EXIF orientation
is applied and metadata is stripped. Animated PNG/WebP and multi-page TIFF are
rejected rather than silently flattening them."#
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        width: None,
        height: None,
        max: None,
        crop: false,
        upscale: false,
        output: None,
        replace: false,
        recursive: false,
        jobs: image_ops::default_jobs(),
        quality: DEFAULT_JPEG_QUALITY,
        yes: false,
        dry_run: false,
        inputs: Vec::new(),
        help: false,
    };
    let mut index = 0;
    while index < args.len() {
        if args[index] == OsStr::new("--") {
            options.inputs.extend(args[index + 1..].iter().cloned());
            break;
        }
        let Some(original) = args[index].to_str() else {
            options.inputs.push(args[index].clone());
            index += 1;
            continue;
        };
        let (option, inline) = original
            .split_once('=')
            .filter(|_| original.starts_with("--"))
            .map_or((original, None), |(key, value)| {
                (key, Some(value.to_owned()))
            });
        let flag = |inline: &Option<String>| -> ToolResult {
            if inline.is_some() {
                Err(ToolError::usage(
                    TOOL,
                    format!("{option} does not take a value"),
                ))
            } else {
                Ok(())
            }
        };
        let value = |index: &mut usize| -> ToolResult<String> {
            if let Some(value) = &inline {
                if value.is_empty() {
                    return Err(ToolError::usage(TOOL, format!("{option} needs a value")));
                }
                return Ok(value.clone());
            }
            common::option_value(TOOL, &args, index, option)
        };
        let path_value = |index: &mut usize| -> ToolResult<OsString> {
            if let Some(value) = &inline {
                if value.is_empty() {
                    return Err(ToolError::usage(TOOL, format!("{option} needs a value")));
                }
                return Ok(OsString::from(value));
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| ToolError::usage(TOOL, format!("{option} needs a value")))
        };
        match option {
            "-h" | "--help" => {
                flag(&inline)?;
                options.help = true;
            }
            "-w" | "--width" => {
                options.width = Some(common::integer(
                    TOOL,
                    &value(&mut index)?,
                    "width",
                    1,
                    65_535,
                )?);
            }
            "-H" | "--height" => {
                options.height = Some(common::integer(
                    TOOL,
                    &value(&mut index)?,
                    "height",
                    1,
                    65_535,
                )?);
            }
            "-m" | "--max" => {
                options.max = Some(common::integer(
                    TOOL,
                    &value(&mut index)?,
                    "maximum dimension",
                    1,
                    65_535,
                )?);
            }
            "--crop" => {
                flag(&inline)?;
                options.crop = true;
            }
            "--upscale" => {
                flag(&inline)?;
                options.upscale = true;
            }
            "-o" | "--output" => {
                options.output = Some(PathBuf::from(path_value(&mut index)?));
            }
            "--replace" => {
                flag(&inline)?;
                options.replace = true;
            }
            "-q" | "--quality" => {
                options.quality = common::integer(TOOL, &value(&mut index)?, "quality", 1, 100)?;
            }
            "-j" | "--jobs" => {
                options.jobs = common::integer(TOOL, &value(&mut index)?, "jobs", 1, 256)? as usize;
            }
            "-r" | "--recursive" => {
                flag(&inline)?;
                options.recursive = true;
            }
            "-y" | "--yes" => {
                flag(&inline)?;
                options.yes = true;
            }
            "-n" | "--dry-run" => {
                flag(&inline)?;
                options.dry_run = true;
            }
            _ if original.starts_with('-') && original != "-" => {
                return Err(ToolError::usage(
                    TOOL,
                    format!("unknown option: {original}"),
                ));
            }
            _ => options.inputs.push(args[index].clone()),
        }
        index += 1;
    }

    if options.output.is_some() && options.replace {
        return Err(ToolError::usage(
            TOOL,
            "--output cannot be combined with --replace",
        ));
    }
    if options.max.is_some() && (options.width.is_some() || options.height.is_some()) {
        return Err(ToolError::usage(
            TOOL,
            "--max cannot be combined with --width or --height",
        ));
    }
    if options.crop && (options.width.is_none() || options.height.is_none()) {
        return Err(ToolError::usage(
            TOOL,
            "--crop requires both --width and --height",
        ));
    }
    if options.width.is_none() && options.height.is_none() && options.max.is_none() {
        options.max = Some(DEFAULT_MAX);
    }
    options.output = image_ops::normalize_output_directory(TOOL, options.output)?;
    Ok(options)
}

fn collect(options: &Options, inputs: &[OsString]) -> ToolResult<CollectedPaths> {
    let collected = common::collect_paths(
        TOOL,
        inputs,
        &InputOptions {
            extensions: image_ops::STILL_EXTENSIONS,
            recursive: options.recursive,
            exclude_directory: options.output.as_deref(),
        },
    )?;
    let files = collected
        .files
        .into_iter()
        .filter(|file| {
            options.output.is_some()
                || options.replace
                || !file
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .is_some_and(|stem| stem.to_ascii_lowercase().ends_with("-resized"))
        })
        .collect();
    Ok(CollectedPaths { files, ..collected })
}

fn resized_name(source: &Path) -> OsString {
    let mut name = source.file_stem().unwrap_or_default().to_os_string();
    name.push("-resized");
    if let Some(extension) = source.extension() {
        name.push(".");
        name.push(extension);
    }
    name
}

fn output_for(source: &Path, options: &Options) -> PathBuf {
    if options.replace {
        return source.to_path_buf();
    }
    if let Some(directory) = &options.output {
        return directory.join(source.file_name().unwrap_or_default());
    }
    source
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(resized_name(source))
}

fn plans(options: &Options, files: Vec<PathBuf>) -> ToolResult<Vec<Plan>> {
    let mut plans: Vec<_> = files
        .into_iter()
        .map(|source| {
            let output = output_for(&source, options);
            let same = common::same_path(&source, &output);
            Plan {
                source,
                output,
                output_exists: false,
                overwrites_source: same,
                removes_source: false,
            }
        })
        .collect();
    common::validate_plans(TOOL, &mut plans)?;
    if !options.replace
        && let Some(plan) = plans.iter().find(|plan| plan.overwrites_source)
    {
        return Err(ToolError::usage(
            TOOL,
            format!(
                "output would overwrite {}; use --replace or another --output directory",
                common::display_path(&plan.source)
            ),
        ));
    }
    Ok(plans)
}

fn target_dimensions(
    source_width: u32,
    source_height: u32,
    options: &Options,
) -> Result<(u32, u32), String> {
    if source_width == 0 || source_height == 0 {
        return Err("image has invalid zero dimensions".into());
    }
    if options.crop {
        let width = options.width.expect("crop width was validated");
        let height = options.height.expect("crop height was validated");
        let scale = (f64::from(width) / f64::from(source_width))
            .max(f64::from(height) / f64::from(source_height));
        if !options.upscale && scale > 1.0 {
            return Err(format!(
                "{}x{} crop would upscale {}x{}; use --upscale or smaller dimensions",
                width, height, source_width, source_height
            ));
        }
        validate_target_size(width, height)?;
        return Ok((width, height));
    }

    let scale = if let Some(maximum) = options.max {
        f64::from(maximum) / f64::from(source_width.max(source_height))
    } else {
        match (options.width, options.height) {
            (Some(width), Some(height)) => (f64::from(width) / f64::from(source_width))
                .min(f64::from(height) / f64::from(source_height)),
            (Some(width), None) => f64::from(width) / f64::from(source_width),
            (None, Some(height)) => f64::from(height) / f64::from(source_height),
            (None, None) => unreachable!("a default maximum is assigned during parsing"),
        }
    };
    let scale = if options.upscale {
        scale
    } else {
        scale.min(1.0)
    };
    let dimensions = (
        (f64::from(source_width) * scale).round().max(1.0) as u32,
        (f64::from(source_height) * scale).round().max(1.0) as u32,
    );
    validate_target_size(dimensions.0, dimensions.1)?;
    Ok(dimensions)
}

fn validate_target_size(width: u32, height: u32) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_OUTPUT_PIXELS {
        return Err(format!(
            "requested output is {}x{} ({:.1} megapixels); the safety limit is {:.0} megapixels",
            width,
            height,
            pixels as f64 / 1_000_000.0,
            MAX_OUTPUT_PIXELS as f64 / 1_000_000.0
        ));
    }
    Ok(())
}

#[derive(Debug)]
enum Status {
    Done {
        before: u64,
        after: u64,
        source_dimensions: (u32, u32),
        target_dimensions: (u32, u32),
    },
    Kept {
        dimensions: (u32, u32),
    },
    Failed(String),
}

fn resize(options: &Options, plan: &Plan) -> Status {
    let metadata = match fs::metadata(&plan.source) {
        Ok(metadata) => metadata,
        Err(error) => return Status::Failed(error.to_string()),
    };
    let image = match image_ops::load_oriented(&plan.source) {
        Ok(image) => image,
        Err(error) => return Status::Failed(error),
    };
    let source_dimensions = (image.width(), image.height());
    let target_dimensions = match target_dimensions(image.width(), image.height(), options) {
        Ok(dimensions) => dimensions,
        Err(error) => return Status::Failed(error),
    };
    if source_dimensions == target_dimensions {
        return Status::Kept {
            dimensions: source_dimensions,
        };
    }
    let resized = if options.crop {
        image.resize_to_fill(
            target_dimensions.0,
            target_dimensions.1,
            FilterType::Lanczos3,
        )
    } else {
        image.resize_exact(
            target_dimensions.0,
            target_dimensions.1,
            FilterType::Lanczos3,
        )
    };
    let temporary = match image_ops::temp_path(TOOL, &plan.output) {
        Ok(path) => path,
        Err(error) => return Status::Failed(error),
    };
    if let Err(error) = image_ops::encode_preserving_format(&resized, &temporary, options.quality) {
        fs::remove_file(&temporary).ok();
        return Status::Failed(error);
    }
    let after = match image_ops::validate_encoded_image(&temporary, target_dimensions, None) {
        Ok(bytes) => bytes,
        Err(error) => {
            fs::remove_file(&temporary).ok();
            return Status::Failed(error);
        }
    };
    let readonly = match image_ops::output_readonly(&plan.source, &plan.output) {
        Ok(readonly) => readonly,
        Err(error) => {
            fs::remove_file(&temporary).ok();
            return Status::Failed(format!("could not inspect output permissions: {error}"));
        }
    };
    if let Err(error) = image_ops::preserve_permissions(&plan.source, &plan.output, &temporary) {
        fs::remove_file(&temporary).ok();
        return Status::Failed(format!("could not preserve output permissions: {error}"));
    }
    if let Err(error) = common::atomic_install(TOOL, &temporary, &plan.output) {
        fs::remove_file(&temporary).ok();
        return Status::Failed(error.message().to_owned());
    }
    if let Err(error) = image_ops::restore_readonly(&plan.output, readonly) {
        return Status::Failed(format!(
            "output installed but read-only state could not be preserved: {error}"
        ));
    }
    Status::Done {
        before: metadata.len(),
        after,
        source_dimensions,
        target_dimensions,
    }
}

fn print_dry_run(options: &Options, plans: &[Plan]) -> ToolResult {
    println!("{TOOL}: dry run — {} file(s)", plans.len());
    for plan in plans.iter().take(100) {
        let image = image_ops::load_oriented(&plan.source).map_err(|error| {
            ToolError::new(
                TOOL,
                format!("{}: {error}", common::display_path(&plan.source)),
            )
        })?;
        let source = (image.width(), image.height());
        let target = target_dimensions(source.0, source.1, options).map_err(|error| {
            ToolError::new(
                TOOL,
                format!("{}: {error}", common::display_path(&plan.source)),
            )
        })?;
        if source == target {
            println!(
                "  keep {} ({}x{}, already within target)",
                common::display_path(&plan.source),
                source.0,
                source.1
            );
        } else {
            let suffix = if plan.overwrites_source {
                " (replace atomically)"
            } else {
                ""
            };
            println!(
                "  {} ({}x{}) -> {} ({}x{}){suffix}",
                common::display_path(&plan.source),
                source.0,
                source.1,
                common::display_path(&plan.output),
                target.0,
                target.1
            );
        }
    }
    if plans.len() > 100 {
        println!("  ... and {} more", plans.len() - 100);
    }
    Ok(())
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    common::init_signals();
    let options = parse(args)?;
    if options.help {
        println!("{}", help());
        return Ok(());
    }
    let mut inputs = options.inputs.clone();
    let mut piped = false;
    if inputs.is_empty() && !common::stdin_is_terminal() {
        inputs = common::parse_input_lines(&common::read_stdin()?);
        piped = !inputs.is_empty();
    }
    if inputs.is_empty() {
        inputs.push(
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .into_os_string(),
        );
    }
    let collected = collect(&options, &inputs)?;
    for warning in &collected.warnings {
        eprintln!("{TOOL}: {warning}");
    }
    if collected.files.is_empty() {
        return Err(ToolError::new(TOOL, "no supported files found"));
    }
    for file in &collected.files {
        image_ops::assert_static(TOOL, file)?;
    }
    let plans = plans(&options, collected.files)?;
    if options.dry_run {
        return print_dry_run(&options, &plans);
    }
    let destructive = plans
        .iter()
        .any(|plan| plan.output_exists || plan.overwrites_source);
    if collected.used_directory
        && !piped
        && destructive
        && !options.yes
        && !common::confirm(
            TOOL,
            &format!(
                "{TOOL}: process {} file(s) and replace existing data",
                plans.len()
            ),
        )?
    {
        return Err(ToolError::cancelled(TOOL));
    }

    let started = Instant::now();
    println!("{TOOL}: {} file(s), {} job(s)", plans.len(), options.jobs);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let results: Vec<Status> = pool.install(|| {
        plans
            .par_iter()
            .map(|plan| {
                if common::interrupted() {
                    Status::Failed("interrupted".into())
                } else {
                    resize(&options, plan)
                }
            })
            .collect()
    });

    let mut done = 0;
    let mut kept = 0;
    let mut failed = 0;
    for (plan, status) in plans.iter().zip(&results) {
        let mapping = if plan.overwrites_source {
            common::display_path(&plan.source)
        } else {
            format!(
                "{} -> {}",
                common::display_path(&plan.source),
                common::display_path(&plan.output)
            )
        };
        match status {
            Status::Done {
                before,
                after,
                source_dimensions,
                target_dimensions,
            } => {
                done += 1;
                println!(
                    "  done    {mapping}  {}x{} -> {}x{}, {} -> {}",
                    source_dimensions.0,
                    source_dimensions.1,
                    target_dimensions.0,
                    target_dimensions.1,
                    common::format_bytes(*before),
                    common::format_bytes(*after)
                );
            }
            Status::Kept { dimensions } => {
                kept += 1;
                println!(
                    "  kept    {} ({}x{}, already within target)",
                    common::display_path(&plan.source),
                    dimensions.0,
                    dimensions.1
                );
            }
            Status::Failed(error) => {
                failed += 1;
                eprintln!("  failed  {}: {error}", common::display_path(&plan.source));
            }
        }
    }
    println!(
        "{TOOL}: {done} done{}{} in {}",
        if kept > 0 {
            format!(", {kept} kept")
        } else {
            String::new()
        },
        if failed > 0 {
            format!(", {failed} failed")
        } else {
            String::new()
        },
        image_ops::duration(started.elapsed())
    );
    if failed > 0 {
        Err(ToolError::with_code(
            TOOL,
            "one or more files could not be completed",
            1,
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        parse(Vec::new()).unwrap()
    }

    #[test]
    fn defaults_to_a_non_upscaling_1920_box() {
        let options = options();
        assert_eq!(options.max, Some(1920));
        assert_eq!(
            target_dimensions(4000, 2000, &options).unwrap(),
            (1920, 960)
        );
        assert_eq!(target_dimensions(800, 600, &options).unwrap(), (800, 600));
    }

    #[test]
    fn width_height_and_crop_dimensions_are_predictable() {
        let width = parse(vec!["--width".into(), "300".into()]).unwrap();
        assert_eq!(target_dimensions(1200, 800, &width).unwrap(), (300, 200));

        let box_options = parse(vec![
            "--width".into(),
            "300".into(),
            "--height".into(),
            "300".into(),
        ])
        .unwrap();
        assert_eq!(
            target_dimensions(1200, 800, &box_options).unwrap(),
            (300, 200)
        );

        let crop = parse(vec![
            "--width".into(),
            "300".into(),
            "--height".into(),
            "300".into(),
            "--crop".into(),
        ])
        .unwrap();
        assert_eq!(target_dimensions(1200, 800, &crop).unwrap(), (300, 300));
    }

    #[test]
    fn crop_refuses_implicit_upscaling() {
        let crop = parse(vec![
            "--width".into(),
            "800".into(),
            "--height".into(),
            "800".into(),
            "--crop".into(),
        ])
        .unwrap();
        assert!(target_dimensions(400, 300, &crop).is_err());
    }

    #[test]
    fn conflicting_options_are_usage_errors() {
        assert!(parse(vec!["--crop".into()]).is_err());
        assert!(
            parse(vec![
                "--max".into(),
                "100".into(),
                "--width".into(),
                "100".into()
            ])
            .is_err()
        );
        assert!(parse(vec!["--replace".into(), "--output".into(), "out".into()]).is_err());
    }

    #[test]
    fn encodes_every_advertised_output_format() {
        let directory = tempfile::tempdir().unwrap();
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            32,
            16,
            image::Rgba([20, 80, 160, 200]),
        ));
        for extension in ["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "qoi"] {
            let output = directory.path().join(format!("image.{extension}"));
            image_ops::encode_preserving_format(&image, &output, DEFAULT_JPEG_QUALITY).unwrap();
            assert_eq!(
                image::image_dimensions(&output).unwrap(),
                (32, 16),
                "failed round-trip for {extension}"
            );
        }
    }

    #[test]
    fn default_output_preserves_non_utf8_safe_components() {
        let source = Path::new("photo.PNG");
        assert_eq!(resized_name(source), OsString::from("photo-resized.PNG"));
    }
}
