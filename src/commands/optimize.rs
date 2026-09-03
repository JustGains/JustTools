use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::codecs::png::{CompressionType, FilterType as PngFilterType, PngEncoder};
use image::{DynamicImage, ImageEncoder, ImageFormat};
use jpeg_encoder::{ChromaSubsamplingMethod, ColorType, Encoder, SamplingFactor};
use rayon::prelude::*;

use crate::common::{self, CollectedPaths, InputOptions, Plan};
use crate::error::{ToolError, ToolResult};

use super::image_ops;

const TOOL: &str = "justoptimize";
const DEFAULT_QUALITY: u8 = 82;

const HELP: &str = r#"justoptimize — Choose the smallest web-ready PNG, WebP, or JPEG.

Usage:
  justoptimize [options] [file-or-folder ...]

Each still image is decoded once and tested as optimized PNG, WebP, and—when
the image is fully opaque—progressive JPEG. The smallest valid result wins.
Images with any transparent pixel are never written as JPEG; PNG and WebP both
preserve their alpha channel.

By default, sources are kept and a smaller result is written beside each source
as <name>-optimized.<chosen-format>. If the original PNG/WebP/JPEG is already
smallest, it is kept and no duplicate is written.

Options:
  -q, --quality N   JPEG/WebP visual quality, 1-100 (default: 82)
  -o, --output DIR  Write <name>.<chosen-format> copies to DIR; keep sources
      --replace     Replace/remove sources only after the winner is installed
  -j, --jobs N      Parallel image evaluations (default: up to 4)
  -r, --recursive   Include nested folders
  -y, --yes         Approve existing-output replacement and destructive batches
  -n, --dry-run     Evaluate formats and show exact paths without final writes
  -h, --help        Show this help

Animated PNG/WebP and multi-page TIFF inputs are rejected rather than flattened.
Encoding is built into JustTools, so JustOptimize needs no external programs.
Run bare for the interactive launcher; its footer shows the headless command."#;

#[derive(Clone, Debug)]
struct Options {
    quality: u8,
    output: Option<PathBuf>,
    replace: bool,
    jobs: usize,
    recursive: bool,
    yes: bool,
    dry_run: bool,
    inputs: Vec<OsString>,
    help: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WebFormat {
    Png,
    Webp,
    Jpeg,
}

impl WebFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Jpeg => "jpg",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Webp => "WebP",
            Self::Jpeg => "JPEG",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Webp => ImageFormat::WebP,
            Self::Jpeg => ImageFormat::Jpeg,
        }
    }

    fn from_path(path: &Path) -> Option<Self> {
        match image_ops::extension(path).as_str() {
            ".png" => Some(Self::Png),
            ".webp" => Some(Self::Webp),
            ".jpg" | ".jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum CandidateFile {
    Original,
    Temporary(PathBuf),
}

impl Drop for CandidateFile {
    fn drop(&mut self) {
        if let Self::Temporary(path) = self {
            fs::remove_file(path).ok();
        }
    }
}

#[derive(Debug)]
struct Candidate {
    format: WebFormat,
    bytes: u64,
    file: CandidateFile,
}

#[derive(Debug)]
struct Evaluation {
    source: PathBuf,
    before: u64,
    dimensions: (u32, u32),
    transparent: bool,
    winner: Candidate,
}

#[derive(Debug)]
struct Prepared {
    evaluation: Evaluation,
    plan: Plan,
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        quality: DEFAULT_QUALITY,
        output: None,
        replace: false,
        jobs: image_ops::default_jobs().min(4),
        recursive: false,
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

fn has_transparency(image: &DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel[3] < 255)
}

fn candidate_path(format: WebFormat) -> Result<PathBuf, String> {
    let temporary = tempfile::Builder::new()
        .prefix("justoptimize-candidate-")
        .suffix(&format!(".{}", format.extension()))
        .tempfile()
        .map_err(|error| error.to_string())?;
    let (file, path) = temporary.keep().map_err(|error| error.error.to_string())?;
    drop(file);
    Ok(path)
}

fn encode_png(image: &DynamicImage, transparent: bool, path: &Path) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    let encoder =
        PngEncoder::new_with_quality(&mut writer, CompressionType::Best, PngFilterType::Adaptive);
    if transparent {
        let rgba = image.to_rgba8();
        encoder
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| error.to_string())?;
    } else {
        let rgb = image.to_rgb8();
        encoder
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn encode_webp(
    image: &DynamicImage,
    transparent: bool,
    quality: u8,
    path: &Path,
) -> Result<(), String> {
    let encoded = if transparent {
        let rgba = image.to_rgba8();
        webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
            .encode(f32::from(quality))
    } else {
        let rgb = image.to_rgb8();
        webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()).encode(f32::from(quality))
    };
    let mut file = BufWriter::new(File::create(path).map_err(|error| error.to_string())?);
    file.write_all(&encoded)
        .map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

fn encode_jpeg(image: &DynamicImage, quality: u8, path: &Path) -> Result<(), String> {
    let rgb = image.to_rgb8();
    let width = u16::try_from(rgb.width())
        .map_err(|_| format!("image width {} exceeds JPEG's 65535 limit", rgb.width()))?;
    let height = u16::try_from(rgb.height())
        .map_err(|_| format!("image height {} exceeds JPEG's 65535 limit", rgb.height()))?;
    let mut writer = BufWriter::new(File::create(path).map_err(|error| error.to_string())?);
    {
        let mut encoder = Encoder::new(&mut writer, quality);
        encoder.set_sampling_factor(if quality >= 90 {
            SamplingFactor::F_1_1
        } else {
            SamplingFactor::F_2_2
        });
        encoder.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
        encoder.set_optimized_huffman_tables(true);
        encoder.set_progressive(true);
        encoder
            .encode(rgb.as_raw(), width, height, ColorType::Rgb)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn encoded_candidate(
    image: &DynamicImage,
    format: WebFormat,
    transparent: bool,
    quality: u8,
) -> Result<Candidate, String> {
    let path = candidate_path(format)?;
    let result = match format {
        WebFormat::Png => encode_png(image, transparent, &path),
        WebFormat::Webp => encode_webp(image, transparent, quality, &path),
        WebFormat::Jpeg => encode_jpeg(image, quality, &path),
    };
    if let Err(error) = result {
        fs::remove_file(&path).ok();
        return Err(format!("{} encoding failed: {error}", format.label()));
    }
    let bytes = image_ops::validate_encoded_image(
        &path,
        (image.width(), image.height()),
        Some(format.image_format()),
    )?;
    Ok(Candidate {
        format,
        bytes,
        file: CandidateFile::Temporary(path),
    })
}

fn evaluate(source: &Path, quality: u8) -> Result<Evaluation, String> {
    let before = fs::metadata(source)
        .map_err(|error| error.to_string())?
        .len();
    let image = image_ops::load_oriented(source)?;
    let dimensions = (image.width(), image.height());
    let transparent = has_transparency(&image);
    let mut candidates = vec![
        encoded_candidate(&image, WebFormat::Png, transparent, quality)?,
        encoded_candidate(&image, WebFormat::Webp, transparent, quality)?,
    ];
    if !transparent {
        candidates.push(encoded_candidate(&image, WebFormat::Jpeg, false, quality)?);
    }
    if let Some(format) = WebFormat::from_path(source) {
        candidates.push(Candidate {
            format,
            bytes: before,
            file: CandidateFile::Original,
        });
    }
    let winner_index = candidates
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| candidate.bytes)
        .map(|(index, _)| index)
        .expect("PNG and WebP candidates are always present");
    let winner = candidates.swap_remove(winner_index);
    Ok(Evaluation {
        source: source.to_path_buf(),
        before,
        dimensions,
        transparent,
        winner,
    })
}

fn output_for(evaluation: &Evaluation, options: &Options) -> PathBuf {
    let source = &evaluation.source;
    if matches!(evaluation.winner.file, CandidateFile::Original) && options.output.is_none() {
        return source.clone();
    }
    let parent = options
        .output
        .as_deref()
        .unwrap_or_else(|| source.parent().unwrap_or_else(|| Path::new(".")));
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let suffix = if options.output.is_none() && !options.replace {
        "-optimized"
    } else {
        ""
    };
    parent.join(format!(
        "{stem}{suffix}.{}",
        evaluation.winner.format.extension()
    ))
}

fn prepare(evaluation: Evaluation, options: &Options) -> Prepared {
    let output = output_for(&evaluation, options);
    let same = common::same_path(&evaluation.source, &output);
    let original_wins = matches!(evaluation.winner.file, CandidateFile::Original);
    let plan = Plan {
        source: evaluation.source.clone(),
        output,
        output_exists: false,
        overwrites_source: same && !original_wins,
        removes_source: options.replace && !same && !original_wins,
    };
    Prepared { evaluation, plan }
}

fn print_choice(prepared: &Prepared, dry_run: bool) {
    let evaluation = &prepared.evaluation;
    let original_wins = matches!(evaluation.winner.file, CandidateFile::Original);
    if original_wins && common::same_path(&evaluation.source, &prepared.plan.output) {
        println!(
            "  kept    {} — original {} is smallest ({})",
            common::display_path(&evaluation.source),
            evaluation.winner.format.label(),
            common::format_bytes(evaluation.before)
        );
        return;
    }
    let action = if dry_run { "would" } else { "chosen" };
    let alpha = if evaluation.transparent {
        ", transparency preserved"
    } else {
        ""
    };
    let suffix = if prepared.plan.removes_source {
        "; then remove source"
    } else if prepared.plan.overwrites_source || prepared.plan.output_exists {
        "; replace target atomically"
    } else {
        "; source kept"
    };
    println!(
        "  {action:<7} {} -> {} — {} {}x{} ({} -> {}{alpha}{suffix})",
        common::display_path(&evaluation.source),
        common::display_path(&prepared.plan.output),
        evaluation.winner.format.label(),
        evaluation.dimensions.0,
        evaluation.dimensions.1,
        common::format_bytes(evaluation.before),
        common::format_bytes(evaluation.winner.bytes),
    );
}

fn install(prepared: &mut Prepared) -> Result<bool, String> {
    if matches!(prepared.evaluation.winner.file, CandidateFile::Original)
        && common::same_path(&prepared.evaluation.source, &prepared.plan.output)
    {
        return Ok(false);
    }
    let temporary = image_ops::temp_path(TOOL, &prepared.plan.output)?;
    let source_candidate = match &prepared.evaluation.winner.file {
        CandidateFile::Temporary(path) => path,
        CandidateFile::Original => &prepared.evaluation.source,
    };
    if let Err(error) = fs::copy(source_candidate, &temporary) {
        fs::remove_file(&temporary).ok();
        return Err(format!("could not stage selected result: {error}"));
    }
    let readonly = image_ops::output_readonly(&prepared.evaluation.source, &prepared.plan.output)
        .map_err(|error| format!("could not inspect output permissions: {error}"))?;
    if let Err(error) = image_ops::preserve_permissions(
        &prepared.evaluation.source,
        &prepared.plan.output,
        &temporary,
    ) {
        fs::remove_file(&temporary).ok();
        return Err(error);
    }
    if let Err(error) = common::atomic_install(TOOL, &temporary, &prepared.plan.output) {
        fs::remove_file(&temporary).ok();
        return Err(error.message().to_owned());
    }
    image_ops::restore_readonly(&prepared.plan.output, readonly)?;
    if prepared.plan.removes_source {
        fs::remove_file(&prepared.evaluation.source).map_err(|error| {
            format!("output installed, but source could not be removed: {error}")
        })?;
    }
    Ok(true)
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    common::init_signals();
    let options = parse(args)?;
    if options.help {
        println!("{HELP}");
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

    let started = Instant::now();
    println!(
        "{TOOL}: evaluating {} file(s) as PNG, WebP, and eligible JPEG",
        collected.files.len()
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let evaluations: Vec<_> = pool.install(|| {
        collected
            .files
            .par_iter()
            .map(|source| evaluate(source, options.quality))
            .collect()
    });
    let mut failures = 0;
    let mut prepared = Vec::new();
    for (source, evaluation) in collected.files.iter().zip(evaluations) {
        match evaluation {
            Ok(evaluation) => prepared.push(prepare(evaluation, &options)),
            Err(error) => {
                failures += 1;
                eprintln!("  failed  {}: {error}", common::display_path(source));
            }
        }
    }
    let mut plans = prepared
        .iter()
        .map(|item| item.plan.clone())
        .collect::<Vec<_>>();
    common::validate_plans(TOOL, &mut plans)?;
    if !options.replace
        && let Some(plan) = plans.iter().find(|plan| plan.overwrites_source)
    {
        return Err(ToolError::usage(
            TOOL,
            format!(
                "--output would overwrite {}; choose another directory or use --replace",
                common::display_path(&plan.source)
            ),
        ));
    }
    for (item, plan) in prepared.iter_mut().zip(plans) {
        item.plan = plan;
    }
    if options.dry_run {
        println!("{TOOL}: dry run — no final files will be written");
        for item in &prepared {
            print_choice(item, true);
        }
        if failures > 0 {
            return Err(ToolError::new(
                TOOL,
                format!("{failures} file(s) could not be evaluated"),
            ));
        }
        return Ok(());
    }

    let destructive = prepared.iter().any(|item| {
        item.plan.output_exists || item.plan.overwrites_source || item.plan.removes_source
    });
    if destructive
        && !options.yes
        && !piped
        && !common::confirm(
            TOOL,
            &format!(
                "{TOOL}: {} selected result(s) will replace existing data; continue",
                prepared.len()
            ),
        )?
    {
        return Err(ToolError::cancelled(TOOL));
    }

    let mut done = 0;
    let mut kept = 0;
    for item in &mut prepared {
        match install(item) {
            Ok(true) => {
                done += 1;
                print_choice(item, false);
            }
            Ok(false) => {
                kept += 1;
                print_choice(item, false);
            }
            Err(error) => {
                failures += 1;
                eprintln!(
                    "  failed  {}: {error}",
                    common::display_path(&item.evaluation.source)
                );
            }
        }
    }
    println!(
        "{TOOL}: {done} written, {kept} already optimal{} in {}",
        if failures > 0 {
            format!(", {failures} failed")
        } else {
            String::new()
        },
        image_ops::duration(started.elapsed())
    );
    if failures > 0 {
        Err(ToolError::with_code(
            TOOL,
            "one or more images could not be optimized",
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
    fn parse_is_source_preserving_by_default() {
        let options = parse(Vec::new()).unwrap();
        assert_eq!(options.quality, 82);
        assert!(!options.replace);
        assert!(options.output.is_none());
    }

    #[test]
    fn output_paths_make_the_policy_obvious() {
        let options = parse(Vec::new()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let candidate = directory.path().join("candidate.webp");
        fs::write(&candidate, b"candidate").unwrap();
        let evaluation = Evaluation {
            source: PathBuf::from("assets/photo.png"),
            before: 100,
            dimensions: (1, 1),
            transparent: false,
            winner: Candidate {
                format: WebFormat::Webp,
                bytes: 50,
                file: CandidateFile::Temporary(candidate),
            },
        };
        assert_eq!(
            output_for(&evaluation, &options),
            PathBuf::from("assets/photo-optimized.webp")
        );
    }

    #[test]
    fn transparency_requires_an_actual_nonopaque_pixel() {
        let opaque = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([1, 2, 3, 255]),
        ));
        let transparent = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([1, 2, 3, 254]),
        ));
        assert!(!has_transparency(&opaque));
        assert!(has_transparency(&transparent));
    }

    #[test]
    fn transparent_image_never_evaluates_to_jpeg() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("alpha.png");
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            3,
            2,
            image::Rgba([20, 40, 80, 100]),
        ))
        .save(&source)
        .unwrap();
        let evaluation = evaluate(&source, 82).unwrap();
        assert!(evaluation.transparent);
        assert_ne!(evaluation.winner.format, WebFormat::Jpeg);
    }
}
