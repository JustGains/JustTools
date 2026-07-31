use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::DynamicImage;
#[cfg(test)]
use image::RgbaImage;
use rayon::prelude::*;

use crate::common::{self, CollectedPaths, InputOptions, Plan};
use crate::error::{ToolError, ToolResult};

use super::image_ops;

const TOOL: &str = "justcrop";

#[derive(Clone, Debug)]
struct Options {
    threshold: u8,
    padding: u32,
    output: Option<PathBuf>,
    replace: bool,
    recursive: bool,
    jobs: usize,
    yes: bool,
    dry_run: bool,
    inputs: Vec<OsString>,
    help: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Bounds {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn help() -> &'static str {
    r#"justcrop — Trim transparent borders to the visible alpha bounds.

Usage:
  justcrop [options] [file-or-folder ...]

Default: include every pixel with nonzero alpha, keep the source, preserve the
format, and write <name>-cropped.<ext>. Fully transparent canvases become a
minimal 1x1 transparent image. Images already at their bounds are unchanged.

Options:
  -t, --threshold N      Ignore alpha values at or below N, 0-254 (default: 0)
  -p, --padding PX       Keep up to PX transparent pixels around the bounds
  -o, --output DIR       Write copies to DIR using the original names
      --replace          Atomically replace source images
  -j, --jobs N           Parallel crops (default: up to 8)
  -r, --recursive        Include nested folders
  -y, --yes              Skip folder replacement confirmation
  -n, --dry-run          Show bounds and outputs without writing
  -h, --help             Show this help

Supported still formats: PNG, WebP, TIFF, and QOI. EXIF orientation is applied
and metadata is stripped. Padding is clamped to the original canvas. Animated
PNG/WebP and multi-page TIFF are rejected rather than silently flattened."#
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        threshold: 0,
        padding: 0,
        output: None,
        replace: false,
        recursive: false,
        jobs: image_ops::default_jobs(),
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
            "-t" | "--threshold" => {
                options.threshold =
                    common::integer(TOOL, &value(&mut index)?, "threshold", 0, 254)? as u8;
            }
            "-p" | "--padding" => {
                options.padding = common::integer(TOOL, &value(&mut index)?, "padding", 0, 65_535)?;
            }
            "-o" | "--output" => {
                options.output = Some(PathBuf::from(path_value(&mut index)?));
            }
            "--replace" => {
                flag(&inline)?;
                options.replace = true;
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
    options.output = image_ops::normalize_output_directory(TOOL, options.output)?;
    Ok(options)
}

fn collect(options: &Options, inputs: &[OsString]) -> ToolResult<CollectedPaths> {
    let collected = common::collect_paths(
        TOOL,
        inputs,
        &InputOptions {
            extensions: image_ops::ALPHA_EXTENSIONS,
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
                    .is_some_and(|stem| stem.to_ascii_lowercase().ends_with("-cropped"))
        })
        .collect();
    Ok(CollectedPaths { files, ..collected })
}

fn cropped_name(source: &Path) -> OsString {
    let mut name = source.file_stem().unwrap_or_default().to_os_string();
    name.push("-cropped");
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
        .join(cropped_name(source))
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

fn scan_alpha_bounds(
    width: u32,
    height: u32,
    padding: u32,
    mut visible: impl FnMut(u32, u32) -> bool,
) -> Result<Bounds, String> {
    if width == 0 || height == 0 {
        return Err("image has invalid zero dimensions".into());
    }

    let mut minimum_x = width;
    let mut minimum_y = height;
    let mut maximum_x = 0;
    let mut maximum_y = 0;
    let mut found = false;
    for y in 0..height {
        for x in 0..width {
            if visible(x, y) {
                found = true;
                minimum_x = minimum_x.min(x);
                minimum_y = minimum_y.min(y);
                maximum_x = maximum_x.max(x);
                maximum_y = maximum_y.max(y);
            }
        }
    }
    if !found {
        return Ok(Bounds {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
    }

    let left = minimum_x.saturating_sub(padding);
    let top = minimum_y.saturating_sub(padding);
    let right = maximum_x
        .saturating_add(1)
        .saturating_add(padding)
        .min(width);
    let bottom = maximum_y
        .saturating_add(1)
        .saturating_add(padding)
        .min(height);
    Ok(Bounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn alpha_bounds(image: &DynamicImage, threshold: u8, padding: u32) -> Result<Bounds, String> {
    match image {
        DynamicImage::ImageLumaA8(buffer) => {
            scan_alpha_bounds(buffer.width(), buffer.height(), padding, |x, y| {
                buffer.get_pixel(x, y)[1] > threshold
            })
        }
        DynamicImage::ImageRgba8(buffer) => {
            scan_alpha_bounds(buffer.width(), buffer.height(), padding, |x, y| {
                buffer.get_pixel(x, y)[3] > threshold
            })
        }
        DynamicImage::ImageLumaA16(buffer) => {
            let threshold = u16::from(threshold) * 257;
            scan_alpha_bounds(buffer.width(), buffer.height(), padding, |x, y| {
                buffer.get_pixel(x, y)[1] > threshold
            })
        }
        DynamicImage::ImageRgba16(buffer) => {
            let threshold = u16::from(threshold) * 257;
            scan_alpha_bounds(buffer.width(), buffer.height(), padding, |x, y| {
                buffer.get_pixel(x, y)[3] > threshold
            })
        }
        DynamicImage::ImageRgba32F(buffer) => {
            let threshold = f32::from(threshold) / 255.0;
            scan_alpha_bounds(buffer.width(), buffer.height(), padding, |x, y| {
                buffer.get_pixel(x, y)[3] > threshold
            })
        }
        _ => scan_alpha_bounds(image.width(), image.height(), padding, |_x, _y| true),
    }
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

fn crop(options: &Options, plan: &Plan) -> Status {
    let metadata = match fs::metadata(&plan.source) {
        Ok(metadata) => metadata,
        Err(error) => return Status::Failed(error.to_string()),
    };
    let image = match image_ops::load_oriented(&plan.source) {
        Ok(image) => image,
        Err(error) => return Status::Failed(error),
    };
    let source_dimensions = (image.width(), image.height());
    let bounds = match alpha_bounds(&image, options.threshold, options.padding) {
        Ok(bounds) => bounds,
        Err(error) => return Status::Failed(error),
    };
    let target_dimensions = (bounds.width, bounds.height);
    if source_dimensions == target_dimensions
        && bounds.x == 0
        && bounds.y == 0
        && options.output.is_none()
    {
        return Status::Kept {
            dimensions: source_dimensions,
        };
    }
    let cropped = image.crop_imm(bounds.x, bounds.y, bounds.width, bounds.height);
    let temporary = match image_ops::temp_path(TOOL, &plan.output) {
        Ok(path) => path,
        Err(error) => return Status::Failed(error),
    };
    if let Err(error) = image_ops::encode_preserving_format(&cropped, &temporary, 85) {
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

fn inspect_bounds(options: &Options, source: &Path) -> ToolResult<((u32, u32), Bounds)> {
    let image = image_ops::load_oriented(source).map_err(|error| {
        ToolError::new(TOOL, format!("{}: {error}", common::display_path(source)))
    })?;
    let dimensions = (image.width(), image.height());
    let bounds = alpha_bounds(&image, options.threshold, options.padding).map_err(|error| {
        ToolError::new(TOOL, format!("{}: {error}", common::display_path(source)))
    })?;
    Ok((dimensions, bounds))
}

fn print_dry_run(options: &Options, plans: &[Plan]) -> ToolResult {
    println!("{TOOL}: dry run — {} file(s)", plans.len());
    for plan in plans.iter().take(100) {
        let (source, bounds) = inspect_bounds(options, &plan.source)?;
        if source == (bounds.width, bounds.height)
            && bounds.x == 0
            && bounds.y == 0
            && options.output.is_none()
        {
            println!(
                "  keep {} ({}x{}, already at alpha bounds)",
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
                bounds.width,
                bounds.height
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
                    crop(&options, plan)
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
                    "  kept    {} ({}x{}, already at alpha bounds)",
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

    #[test]
    fn finds_alpha_bounds_and_applies_clamped_padding() {
        let mut image = RgbaImage::from_pixel(10, 8, image::Rgba([0, 0, 0, 0]));
        for y in 2..6 {
            for x in 3..8 {
                image.put_pixel(x, y, image::Rgba([20, 80, 160, 255]));
            }
        }
        let image = DynamicImage::ImageRgba8(image);
        assert_eq!(
            alpha_bounds(&image, 0, 0).unwrap(),
            Bounds {
                x: 3,
                y: 2,
                width: 5,
                height: 4
            }
        );
        assert_eq!(
            alpha_bounds(&image, 0, 3).unwrap(),
            Bounds {
                x: 0,
                y: 0,
                width: 10,
                height: 8
            }
        );
    }

    #[test]
    fn threshold_and_empty_canvas_are_predictable() {
        let mut image = RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        image.put_pixel(3, 2, image::Rgba([255, 255, 255, 8]));
        let image = DynamicImage::ImageRgba8(image);
        assert_eq!(
            alpha_bounds(&image, 0, 0).unwrap(),
            Bounds {
                x: 3,
                y: 2,
                width: 1,
                height: 1
            }
        );
        assert_eq!(
            alpha_bounds(&image, 8, 0).unwrap(),
            Bounds {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        );
    }

    #[test]
    fn nonzero_sixteen_bit_alpha_is_not_rounded_away() {
        let mut image = image::ImageBuffer::from_pixel(4, 4, image::Rgba([0_u16, 0, 0, 0]));
        image.put_pixel(3, 2, image::Rgba([65_535, 20_000, 10_000, 1]));
        let image = DynamicImage::ImageRgba16(image);
        assert_eq!(
            alpha_bounds(&image, 0, 0).unwrap(),
            Bounds {
                x: 3,
                y: 2,
                width: 1,
                height: 1
            }
        );
    }

    #[test]
    fn default_output_preserves_non_utf8_safe_components() {
        let source = Path::new("icon.PNG");
        assert_eq!(cropped_name(source), OsString::from("icon-cropped.PNG"));
    }

    #[test]
    fn conflicting_options_are_usage_errors() {
        assert!(parse(vec!["--replace".into(), "--output".into(), "out".into()]).is_err());
        assert!(parse(vec!["--threshold".into(), "255".into()]).is_err());
    }
}
