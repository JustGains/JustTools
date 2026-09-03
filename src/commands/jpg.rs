use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{DynamicImage, ImageFormat, RgbImage};
use jpeg_encoder::{ChromaSubsamplingMethod, ColorType, Encoder, SamplingFactor};
use rayon::prelude::*;

use crate::common::{self, CollectedPaths, InputOptions, Plan};
use crate::error::{ToolError, ToolResult};

use super::image_ops;

const TOOL: &str = "justjpg";
const DEFAULT_QUALITY: u8 = 85;
const DEFAULT_BACKGROUND: [u8; 3] = [255, 255, 255];

#[derive(Clone, Debug)]
struct Options {
    quality: u8,
    background: [u8; 3],
    baseline: bool,
    output: Option<PathBuf>,
    replace: bool,
    recursive: bool,
    jobs: usize,
    yes: bool,
    dry_run: bool,
    inputs: Vec<OsString>,
    help: bool,
}

fn help() -> &'static str {
    r#"justjpg — Create compact, web-ready JPEGs with native Rust encoding.

Usage:
  justjpg [options] [file-or-folder ...]

Default: quality 85, progressive 4:2:0 JPEG, box-averaged chroma, optimized
Huffman tables, white alpha background, stripped metadata, and source kept.
Beside-source outputs are named <name>-optimized.jpg; --output writes
<DIR>/<name>.jpg and keeps sources. Existing destinations are replaced
atomically. Run bare to open the interactive launcher; explicit arguments
bypass the UI.

Options:
  -q, --quality N        JPEG quality, 1-100 (default: 85)
  -b, --background COLOR Alpha background: white, black, or RRGGBB
      --baseline         Write baseline rather than progressive JPEG
  -o, --output DIR       Write <name>.jpg copies to DIR
      --replace          Replace JPEGs; convert and remove other source formats
  -j, --jobs N           Parallel encodes (default: up to 4)
  -r, --recursive        Include nested folders
  -y, --yes              Skip destructive folder confirmation
  -n, --dry-run          Show inputs and outputs without writing
  -h, --help             Show this help

Supported still inputs: JPEG, PNG, WebP, BMP, TIFF, and QOI. EXIF orientation
is applied. Animated PNG/WebP and multi-page TIFF are rejected rather than
silently flattened. JPEG is lossy; use --quality 90+ when detail matters more
than file size; quality 90+ automatically uses full 4:4:4 chroma."#
}

fn parse_color(value: &str) -> Result<[u8; 3], String> {
    match value.to_ascii_lowercase().as_str() {
        "white" => return Ok([255, 255, 255]),
        "black" => return Ok([0, 0, 0]),
        _ => {}
    }
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("background must be white, black, or a six-digit RRGGBB color".into());
    }
    Ok([
        u8::from_str_radix(&hex[0..2], 16).expect("validated hex"),
        u8::from_str_radix(&hex[2..4], 16).expect("validated hex"),
        u8::from_str_radix(&hex[4..6], 16).expect("validated hex"),
    ])
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        quality: DEFAULT_QUALITY,
        background: DEFAULT_BACKGROUND,
        baseline: false,
        output: None,
        replace: false,
        recursive: false,
        jobs: image_ops::default_jobs().min(4),
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
            "-q" | "--quality" => {
                options.quality =
                    common::integer(TOOL, &value(&mut index)?, "quality", 1, 100)? as u8;
            }
            "-b" | "--background" => {
                options.background = parse_color(&value(&mut index)?)
                    .map_err(|error| ToolError::usage(TOOL, error))?;
            }
            "--baseline" => {
                flag(&inline)?;
                options.baseline = true;
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
                    .is_some_and(|stem| stem.to_ascii_lowercase().ends_with("-optimized"))
        })
        .collect();
    Ok(CollectedPaths { files, ..collected })
}

fn jpg_name(source: &Path, optimized_suffix: bool) -> OsString {
    let mut name = source.file_stem().unwrap_or_default().to_os_string();
    if optimized_suffix {
        name.push("-optimized");
    }
    name.push(".jpg");
    name
}

fn output_for(source: &Path, options: &Options) -> PathBuf {
    let parent = options
        .output
        .as_deref()
        .unwrap_or_else(|| source.parent().unwrap_or_else(|| Path::new(".")));
    if options.replace && matches!(image_ops::extension(source).as_str(), ".jpg" | ".jpeg") {
        source.to_path_buf()
    } else {
        parent.join(jpg_name(
            source,
            options.output.is_none() && !options.replace,
        ))
    }
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
                removes_source: options.replace && !same,
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
    if options.replace
        && let Some(plan) = plans
            .iter()
            .find(|plan| plan.removes_source && plan.output_exists)
    {
        return Err(ToolError::usage(
            TOOL,
            format!(
                "replacement target already exists: {}; use --output or move it first",
                common::display_path(&plan.output)
            ),
        ));
    }
    Ok(plans)
}

fn flatten_alpha(image: &DynamicImage, background: [u8; 3]) -> RgbImage {
    if let DynamicImage::ImageRgb8(rgb) = image {
        return rgb.clone();
    }
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let owned;
    let rgba = if let DynamicImage::ImageRgba8(rgba) = image {
        rgba
    } else {
        owned = image.to_rgba8();
        &owned
    };
    let mut output = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.pixels() {
        let alpha = u16::from(pixel[3]);
        let inverse = 255 - alpha;
        for channel in 0..3 {
            let blended = (u16::from(pixel[channel]) * alpha
                + u16::from(background[channel]) * inverse
                + 127)
                / 255;
            output.push(blended as u8);
        }
    }
    RgbImage::from_raw(rgba.width(), rgba.height(), output)
        .expect("RGB allocation matches source dimensions")
}

fn encode_jpg(image: &RgbImage, output: &Path, quality: u8, baseline: bool) -> Result<(), String> {
    let width = u16::try_from(image.width())
        .map_err(|_| format!("image width {} exceeds JPEG's 65535 limit", image.width()))?;
    let height = u16::try_from(image.height())
        .map_err(|_| format!("image height {} exceeds JPEG's 65535 limit", image.height()))?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    {
        let mut encoder = Encoder::new(&mut writer, quality);
        encoder.set_sampling_factor(if quality >= 90 {
            SamplingFactor::F_1_1
        } else {
            SamplingFactor::F_2_2
        });
        encoder.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
        encoder.set_optimized_huffman_tables(true);
        encoder.set_progressive(!baseline);
        encoder
            .encode(image.as_raw(), width, height, ColorType::Rgb)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

#[derive(Debug)]
enum Status {
    Done {
        before: u64,
        after: u64,
        dimensions: (u32, u32),
    },
    Warning {
        before: u64,
        after: u64,
        dimensions: (u32, u32),
        message: String,
    },
    Failed(String),
}

fn convert(options: &Options, plan: &Plan) -> Status {
    let metadata = match fs::metadata(&plan.source) {
        Ok(metadata) => metadata,
        Err(error) => return Status::Failed(error.to_string()),
    };
    let image = match image_ops::load_oriented(&plan.source) {
        Ok(image) => image,
        Err(error) => return Status::Failed(error),
    };
    let dimensions = (image.width(), image.height());
    let rgb = flatten_alpha(&image, options.background);
    let temporary = match image_ops::temp_path(TOOL, &plan.output) {
        Ok(path) => path,
        Err(error) => return Status::Failed(error),
    };
    if let Err(error) = encode_jpg(&rgb, &temporary, options.quality, options.baseline) {
        fs::remove_file(&temporary).ok();
        return Status::Failed(error);
    }
    let after =
        match image_ops::validate_encoded_image(&temporary, dimensions, Some(ImageFormat::Jpeg)) {
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
    if plan.removes_source
        && let Err(error) = fs::remove_file(&plan.source)
    {
        return Status::Warning {
            before: metadata.len(),
            after,
            dimensions,
            message: format!("output created, but source could not be removed: {error}"),
        };
    }
    Status::Done {
        before: metadata.len(),
        after,
        dimensions,
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
        let suffix = if plan.removes_source {
            " (then remove source)"
        } else if plan.overwrites_source {
            " (replace atomically)"
        } else {
            ""
        };
        println!(
            "  {} ({}x{}) -> {}{suffix}",
            common::display_path(&plan.source),
            image.width(),
            image.height(),
            common::display_path(&plan.output)
        );
    }
    if plans.len() > 100 {
        println!("  ... and {} more", plans.len() - 100);
    }
    let mode = if options.baseline {
        "baseline"
    } else {
        "progressive"
    };
    let sampling = if options.quality >= 90 {
        "4:4:4"
    } else {
        "4:2:0"
    };
    println!(
        "  settings: quality {}, {mode}, {sampling}, background #{:02X}{:02X}{:02X}",
        options.quality, options.background[0], options.background[1], options.background[2]
    );
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
        .any(|plan| plan.output_exists || plan.overwrites_source || plan.removes_source);
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
                    convert(&options, plan)
                }
            })
            .collect()
    });

    let mut done = 0;
    let mut warnings = 0;
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
                dimensions,
            } => {
                done += 1;
                println!(
                    "  done    {mapping}  {}x{}, {} -> {}",
                    dimensions.0,
                    dimensions.1,
                    common::format_bytes(*before),
                    common::format_bytes(*after)
                );
            }
            Status::Warning {
                before,
                after,
                dimensions,
                message,
            } => {
                warnings += 1;
                eprintln!(
                    "  warning {mapping}  {}x{}, {} -> {}: {message}",
                    dimensions.0,
                    dimensions.1,
                    common::format_bytes(*before),
                    common::format_bytes(*after)
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
        if warnings > 0 {
            format!(", {warnings} warning")
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
    if failed > 0 || warnings > 0 {
        Err(ToolError::with_code(
            TOOL,
            "one or more files could not be completed as requested",
            1,
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sof_sampling(bytes: &[u8], marker: u8) -> Option<u8> {
        let index = bytes
            .windows(2)
            .position(|candidate| candidate == [0xff, marker])?;
        bytes.get(index + 11).copied()
    }

    #[test]
    fn parses_defaults_and_colors() {
        let defaults = parse(Vec::new()).unwrap();
        assert_eq!(defaults.quality, 85);
        assert_eq!(defaults.background, [255, 255, 255]);
        assert!(!defaults.baseline);
        assert_eq!(parse_color("#12aBef").unwrap(), [0x12, 0xab, 0xef]);
        assert_eq!(parse_color("black").unwrap(), [0, 0, 0]);
        assert!(parse_color("12345").is_err());
    }

    #[test]
    fn alpha_is_composited_instead_of_discarded() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 0, 0, 128]),
        ));
        assert_eq!(
            flatten_alpha(&image, [255, 255, 255]).get_pixel(0, 0).0,
            [255, 127, 127]
        );
        assert_eq!(
            flatten_alpha(&image, [0, 0, 0]).get_pixel(0, 0).0,
            [128, 0, 0]
        );
    }

    #[test]
    fn output_names_are_source_safe() {
        assert_eq!(
            jpg_name(Path::new("portrait.PNG"), true),
            OsString::from("portrait-optimized.jpg")
        );
        assert_eq!(
            jpg_name(Path::new("portrait.PNG"), false),
            OsString::from("portrait.jpg")
        );
    }

    #[test]
    fn encoder_round_trips_a_real_jpeg() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("image.jpg");
        let image = RgbImage::from_pixel(37, 19, image::Rgb([20, 80, 160]));
        encode_jpg(&image, &output, 85, false).unwrap();
        assert_eq!(image::image_dimensions(&output).unwrap(), (37, 19));
        let bytes = fs::read(&output).unwrap();
        assert!(
            bytes.windows(2).any(|marker| marker == [0xff, 0xc2]),
            "default encoder output was not progressive"
        );
        assert_eq!(sof_sampling(&bytes, 0xc2), Some(0x22));
    }

    #[test]
    fn high_quality_uses_full_chroma_and_baseline_is_decodable() {
        let directory = tempfile::tempdir().unwrap();
        let image = RgbImage::from_pixel(37, 19, image::Rgb([20, 80, 160]));

        let progressive = directory.path().join("quality-90.jpg");
        encode_jpg(&image, &progressive, 90, false).unwrap();
        let progressive_bytes = fs::read(&progressive).unwrap();
        assert_eq!(sof_sampling(&progressive_bytes, 0xc2), Some(0x11));

        let baseline = directory.path().join("baseline.jpg");
        encode_jpg(&image, &baseline, 85, true).unwrap();
        let baseline_bytes = fs::read(&baseline).unwrap();
        assert_eq!(sof_sampling(&baseline_bytes, 0xc0), Some(0x22));
        assert!(
            !baseline_bytes
                .windows(2)
                .any(|marker| marker == [0xff, 0xc2])
        );
        assert_eq!(image::image_dimensions(baseline).unwrap(), (37, 19));
    }

    #[test]
    fn conflicting_and_invalid_options_are_usage_errors() {
        assert!(parse(vec!["--replace".into(), "--output".into(), "out".into()]).is_err());
        assert!(parse(vec!["--quality".into(), "0".into()]).is_err());
        assert!(parse(vec!["--background".into(), "not-a-color".into()]).is_err());
    }
}
