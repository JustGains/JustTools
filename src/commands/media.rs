use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use rayon::prelude::*;

use crate::common::{self, CollectedPaths, InputOptions, Plan};
use crate::deps;
use crate::error::{ToolError, ToolResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaMode {
    Png,
    Webp,
    Video,
    Avif,
    Audio,
    Mp3,
    Wav,
}

impl MediaMode {
    fn tool(self) -> &'static str {
        match self {
            Self::Png => "justpng",
            Self::Webp => "justwebp",
            Self::Video => "justvideo",
            Self::Avif => "justavif",
            Self::Audio => "justaudio",
            Self::Mp3 => "justmp3",
            Self::Wav => "justwav",
        }
    }
    fn dependency(self) -> &'static str {
        match self {
            Self::Png => "pngquant",
            Self::Webp => "cwebp",
            _ => "ffmpeg",
        }
    }
    fn extensions(self, include_target: bool) -> Vec<&'static str> {
        match self {
            Self::Png => vec![".png"],
            Self::Webp => {
                let mut values = vec![".jpg", ".jpeg", ".png", ".bmp", ".tif", ".tiff"];
                if include_target {
                    values.push(".webp");
                }
                values
            }
            Self::Avif => {
                let mut values = vec![".jpg", ".jpeg", ".png", ".webp", ".bmp", ".tif", ".tiff"];
                if include_target {
                    values.push(".avif");
                }
                values
            }
            Self::Video => vec![
                ".mp4", ".m4v", ".mov", ".mkv", ".avi", ".webm", ".wmv", ".flv", ".mpg", ".mpeg",
                ".ts", ".m2ts", ".mts", ".3gp", ".ogv",
            ],
            Self::Audio | Self::Mp3 | Self::Wav => vec![
                ".aac", ".ac3", ".aiff", ".alac", ".ape", ".avi", ".flac", ".m4a", ".m4v", ".mka",
                ".mkv", ".mov", ".mp2", ".mp3", ".mp4", ".mpeg", ".mpg", ".ogg", ".opus", ".ts",
                ".wav", ".webm", ".wma", ".wmv",
            ],
        }
    }
    fn target_extension(self) -> &'static str {
        match self {
            Self::Png => ".png",
            Self::Webp => ".webp",
            Self::Video => ".mp4",
            Self::Avif => ".avif",
            Self::Audio => ".m4a",
            Self::Mp3 => ".mp3",
            Self::Wav => ".wav",
        }
    }
    fn default_jobs(self) -> usize {
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        match self {
            Self::Png => cpus.clamp(1, 8),
            Self::Webp => (cpus / 2).clamp(1, 4),
            _ => (cpus / 4).clamp(1, 2),
        }
    }
    fn is_audio(self) -> bool {
        matches!(self, Self::Audio | Self::Mp3 | Self::Wav)
    }
}

#[derive(Clone, Debug)]
struct Options {
    output: Option<PathBuf>,
    jobs: usize,
    recursive: bool,
    yes: bool,
    dry_run: bool,
    replace: bool,
    include_target: bool,
    quality: u32,
    quality_range: String,
    speed: u32,
    method: u32,
    crf: u32,
    preset: String,
    audio_bitrate: String,
    sample_rate: u32,
    bits: u32,
    inputs: Vec<OsString>,
    help: bool,
}

fn help(mode: MediaMode) -> String {
    let help = match mode {
        MediaMode::Png => format!(
            r#"justpng — Optimize PNG files quickly with pngquant.

Usage:
  justpng [options] [file-or-folder ...]

With no output folder, each source PNG is replaced atomically only when the
result is smaller. --output writes <DIR>/<same-name>.png, keeps the source, and
atomically replaces an existing destination. Folders scan direct children
unless --recursive is used. Animated PNGs are rejected.

Options:
  -q, --quality MIN-MAX  Quality range (default: 65-90)
  -s, --speed N          Speed, 1=best to 11=fastest (default: 3)
  -o, --output DIR       Write copies to DIR and keep sources
  -j, --jobs N           Parallel encodes (default: {})
  -r, --recursive        Include nested folders
  -y, --yes              Skip folder-scan confirmation
  -n, --dry-run          Preview without changing files
  -h, --help             Show this help"#,
            mode.default_jobs()
        ),
        MediaMode::Webp => format!(
            r#"justwebp — Convert images to compact lossy WebP files.

Usage:
  justwebp [options] [file-or-folder ...]

Default quality 82/method 5. With no output folder, the destination is
<source-folder>/<name>.webp; an existing destination is replaced and the source
is removed only after a smaller WebP is safely installed. --output writes
<DIR>/<name>.webp and always keeps the source. Animated PNG/WebP and multi-page
TIFF are rejected.

Options:
  -q, --quality N        Quality, 0-100 (default: 82)
  -m, --method N         Compression method, 0-6 (default: 5)
  -w, --include-webp     Re-encode WebP sources
  -o, --output DIR       Write copies to DIR and keep sources
  -j, --jobs N           Parallel encodes (default: {})
  -r, --recursive        Include nested folders
  -y, --yes              Skip folder-scan confirmation
  -n, --dry-run          Preview without changing files
  -h, --help             Show this help"#,
            mode.default_jobs()
        ),
        MediaMode::Video => format!(
            r#"justvideo — Optimize videos as streaming-ready 720p H.264 MP4.

Usage:
  justvideo [options] [file-or-folder ...]

Default output is <name>-web.mp4 using CRF 28, x264 medium, AAC 128k.

Options:
  -f, --replace          Replace sources; non-MP4 sources become <name>.mp4
  -o, --output DIR       Write outputs to DIR and keep sources
  -j, --jobs N           Parallel encodes (default: {})
  -r, --recursive        Include nested folders
  -y, --yes              Skip folder-scan confirmation
  -n, --dry-run          Preview without changing files
      --crf N            H.264 quality, 0-51 (default: 28)
      --preset NAME      x264 preset (default: medium)
      --audio-bitrate N  AAC bitrate (default: 128k)
  -h, --help             Show this help"#,
            mode.default_jobs()
        ),
        MediaMode::Avif => format!(
            r#"justavif — Convert still images to compact AVIF files.

Usage:
  justavif [options] [file-or-folder ...]

Defaults: AV1 quality 60, speed 6, 4:2:0, stripped metadata. With no output
folder, the destination is <source-folder>/<name>.avif; an existing destination
is replaced and the source is removed only if the AVIF is smaller. --output
writes <DIR>/<name>.avif and keeps the source. Transparent, animated, and
multi-page inputs are rejected rather than losing information.

Options:
  -q, --quality N        Visual quality, 0-100 (default: 60)
  -s, --speed N          Encoder speed, 0-8 (default: 6)
      --include-avif     Re-encode AVIF sources
  -o, --output DIR       Write copies to DIR and keep sources
  -j, --jobs N           Parallel encodes (default: {})
  -r, --recursive        Include nested folders
  -y, --yes              Skip folder-scan confirmation
  -n, --dry-run          Preview without changing files
  -h, --help             Show this help"#,
            mode.default_jobs()
        ),
        MediaMode::Audio | MediaMode::Mp3 | MediaMode::Wav => {
            let (description, specific) = match mode {
                MediaMode::Audio => (
                    "AAC-LC M4A at 160 kb/s, 48 kHz",
                    "      --bitrate RATE     AAC bitrate (default: 160k)",
                ),
                MediaMode::Mp3 => (
                    "LAME VBR quality 2, 48 kHz",
                    "  -q, --quality N       LAME VBR quality, 0-9 (default: 2)",
                ),
                MediaMode::Wav => (
                    "PCM 16-bit, 48 kHz, stereo",
                    "      --bits N          PCM depth, 16 or 24 (default: 16)",
                ),
                _ => unreachable!(),
            };
            format!(
                r#"{} — Convert audio or extract it from video.

Usage:
  {} [options] [file-or-folder ...]

Default: {description}. Sources are kept.

Options:
  -f, --replace          Remove source after output is safely installed
  -o, --output DIR       Write outputs to DIR and keep sources
  -j, --jobs N           Parallel conversions (default: {})
  -r, --recursive        Include nested folders
  -y, --yes              Skip folder-scan confirmation
  -n, --dry-run          Preview without changing files
      --reencode         Include files already in the target format
      --sample-rate N    Output sample rate (default: 48000)
{specific}
  -h, --help             Show this help"#,
                mode.tool(),
                mode.tool(),
                mode.default_jobs()
            )
        }
    };
    format!(
        "{help}\n\nRun {} with no arguments to open the interactive launcher. Its Headless\nfooter shows the equivalent direct command; explicit arguments and pipes bypass the UI.",
        mode.tool()
    )
}

fn env_u32(name: &str, fallback: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn parse(mode: MediaMode, args: Vec<OsString>) -> ToolResult<Options> {
    let tool = mode.tool();
    let mut options = Options {
        output: None,
        jobs: std::env::var("JOBS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| mode.default_jobs()),
        recursive: false,
        yes: false,
        dry_run: false,
        replace: false,
        include_target: false,
        quality: if mode == MediaMode::Avif {
            env_u32("QUALITY", 60)
        } else if mode == MediaMode::Mp3 {
            2
        } else {
            env_u32("QUALITY", 82)
        },
        quality_range: std::env::var("QUALITY").unwrap_or_else(|_| "65-90".into()),
        speed: env_u32("SPEED", if mode == MediaMode::Avif { 6 } else { 3 }),
        method: env_u32("METHOD", 5),
        crf: env_u32("CRF", 28),
        preset: std::env::var("PRESET").unwrap_or_else(|_| "medium".into()),
        audio_bitrate: std::env::var("AUDIO_BR").unwrap_or_else(|_| {
            if mode == MediaMode::Audio {
                "160k".into()
            } else {
                "128k".into()
            }
        }),
        sample_rate: 48_000,
        bits: 16,
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
                    tool,
                    format!("{option} does not take a value"),
                ))
            } else {
                Ok(())
            }
        };
        let value = |index: &mut usize| -> ToolResult<String> {
            if let Some(value) = &inline {
                if value.is_empty() {
                    return Err(ToolError::usage(tool, format!("{option} needs a value")));
                }
                return Ok(value.clone());
            }
            common::option_value(tool, &args, index, option)
        };
        let path_value = |index: &mut usize| -> ToolResult<OsString> {
            if let Some(value) = &inline {
                if value.is_empty() {
                    return Err(ToolError::usage(tool, format!("{option} needs a value")));
                }
                return Ok(OsString::from(value));
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| ToolError::usage(tool, format!("{option} needs a value")))
        };
        match option {
            "-h" | "--help" => {
                flag(&inline)?;
                options.help = true;
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
            "-o" | "--output" => options.output = Some(PathBuf::from(path_value(&mut index)?)),
            "-j" | "--jobs" => {
                options.jobs = common::integer(tool, &value(&mut index)?, "jobs", 1, 256)? as usize
            }
            "-f" | "--replace" if mode == MediaMode::Video || mode.is_audio() => {
                flag(&inline)?;
                options.replace = true;
            }
            "-f" | "--force"
                if matches!(mode, MediaMode::Png | MediaMode::Webp | MediaMode::Avif) =>
            {
                flag(&inline)?;
                options.yes = true;
            }
            "-q" | "--quality" if mode == MediaMode::Png => {
                options.quality_range = value(&mut index)?
            }
            "-q" | "--quality"
                if matches!(mode, MediaMode::Webp | MediaMode::Avif | MediaMode::Mp3) =>
            {
                options.quality = common::integer(
                    tool,
                    &value(&mut index)?,
                    "quality",
                    0,
                    if mode == MediaMode::Mp3 { 9 } else { 100 },
                )?
            }
            "-s" | "--speed" if matches!(mode, MediaMode::Png | MediaMode::Avif) => {
                options.speed = common::integer(
                    tool,
                    &value(&mut index)?,
                    "speed",
                    if mode == MediaMode::Png { 1 } else { 0 },
                    if mode == MediaMode::Png { 11 } else { 8 },
                )?
            }
            "-m" | "--method" if mode == MediaMode::Webp => {
                options.method = common::integer(tool, &value(&mut index)?, "method", 0, 6)?
            }
            "-w" | "--include-webp" if mode == MediaMode::Webp => {
                flag(&inline)?;
                options.include_target = true;
            }
            "--include-avif" if mode == MediaMode::Avif => {
                flag(&inline)?;
                options.include_target = true;
            }
            "--reencode" if mode.is_audio() => {
                flag(&inline)?;
                options.include_target = true;
            }
            "--sample-rate" if mode.is_audio() => {
                options.sample_rate =
                    common::integer(tool, &value(&mut index)?, "sample rate", 8_000, 192_000)?
            }
            "--bits" if mode == MediaMode::Wav => {
                options.bits = common::integer(tool, &value(&mut index)?, "bits", 16, 24)?;
                if !matches!(options.bits, 16 | 24) {
                    return Err(ToolError::usage(tool, "bits must be 16 or 24"));
                }
            }
            "--bitrate" if mode == MediaMode::Audio => {
                options.audio_bitrate = value(&mut index)?;
                validate_bitrate(tool, &options.audio_bitrate)?;
            }
            "--crf" if mode == MediaMode::Video => {
                options.crf = common::integer(tool, &value(&mut index)?, "CRF", 0, 51)?
            }
            "--preset" if mode == MediaMode::Video => options.preset = value(&mut index)?,
            "--audio-bitrate" if mode == MediaMode::Video => {
                options.audio_bitrate = value(&mut index)?;
                validate_bitrate(tool, &options.audio_bitrate)?;
            }
            _ if original.starts_with('-') && original != "-" => {
                return Err(ToolError::usage(
                    tool,
                    format!("unknown option: {original}"),
                ));
            }
            _ => options.inputs.push(args[index].clone()),
        }
        index += 1;
    }
    if options.jobs == 0 || options.jobs > 256 {
        return Err(ToolError::usage(
            tool,
            "jobs must be an integer from 1 to 256",
        ));
    }
    if mode == MediaMode::Png {
        options.quality_range = validate_quality_range(tool, &options.quality_range)?;
        if !(1..=11).contains(&options.speed) {
            return Err(ToolError::usage(
                tool,
                "speed must be an integer from 1 to 11",
            ));
        }
    }
    if mode == MediaMode::Webp {
        if options.quality > 100 {
            return Err(ToolError::usage(
                tool,
                "quality must be an integer from 0 to 100",
            ));
        }
        if options.method > 6 {
            return Err(ToolError::usage(
                tool,
                "method must be an integer from 0 to 6",
            ));
        }
    }
    if mode == MediaMode::Avif && (options.quality > 100 || options.speed > 8) {
        return Err(ToolError::usage(
            tool,
            "quality must be 0-100 and speed must be 0-8",
        ));
    }
    if mode == MediaMode::Video {
        let allowed = [
            "ultrafast",
            "superfast",
            "veryfast",
            "faster",
            "fast",
            "medium",
            "slow",
            "slower",
            "veryslow",
            "placebo",
        ];
        if options.crf > 51 || !allowed.contains(&options.preset.as_str()) {
            return Err(ToolError::usage(
                tool,
                format!("preset must be one of: {}", allowed.join(", ")),
            ));
        }
    }
    if options.output.is_some() && options.replace && mode.is_audio() {
        return Err(ToolError::usage(
            tool,
            "--replace cannot be combined with --output",
        ));
    }
    if matches!(mode, MediaMode::Video | MediaMode::Audio) {
        validate_bitrate(tool, &options.audio_bitrate)?;
    }
    if let Some(output) = &options.output {
        options.output = Some(fs::canonicalize(output).unwrap_or_else(|_| {
            if output.is_absolute() {
                output.clone()
            } else {
                std::env::current_dir().unwrap_or_default().join(output)
            }
        }));
    }
    Ok(options)
}

fn validate_quality_range(tool: &str, value: &str) -> ToolResult<String> {
    if let Ok(maximum) = value.parse::<u32>()
        && maximum <= 100
    {
        return Ok(format!("0-{maximum}"));
    }
    let mut parts = value.split('-');
    let first = parts.next().and_then(|part| part.parse::<u32>().ok());
    let second = parts.next().and_then(|part| part.parse::<u32>().ok());
    if parts.next().is_some()
        || first.is_none()
        || second.is_none()
        || first > second
        || second > Some(100)
    {
        return Err(ToolError::usage(
            tool,
            "quality must be MIN-MAX with 0 <= MIN <= MAX <= 100",
        ));
    }
    Ok(value.to_owned())
}

fn validate_bitrate(tool: &str, value: &str) -> ToolResult {
    let body = value.strip_suffix(['k', 'K', 'm', 'M']).unwrap_or(value);
    if body.parse::<f64>().is_err() {
        return Err(ToolError::usage(tool, "bitrate must look like 160k"));
    }
    Ok(())
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|value| format!(".{}", value.to_ascii_lowercase()))
        .unwrap_or_default()
}

fn collect(mode: MediaMode, options: &Options, inputs: &[OsString]) -> ToolResult<CollectedPaths> {
    let extensions = mode.extensions(options.include_target);
    let collected = common::collect_paths(
        mode.tool(),
        inputs,
        &InputOptions {
            extensions: &extensions,
            recursive: options.recursive,
            exclude_directory: options.output.as_deref(),
        },
    )?;
    if mode.is_audio() && !options.include_target {
        for input in inputs {
            let path = PathBuf::from(input);
            if path.is_file() && extension(&path) == mode.target_extension() {
                return Err(ToolError::new(
                    mode.tool(),
                    format!(
                        "{} is already {}; use --reencode",
                        common::display_path(&path),
                        mode.target_extension()[1..].to_ascii_uppercase()
                    ),
                ));
            }
        }
    }
    Ok(CollectedPaths {
        files: collected
            .files
            .into_iter()
            .filter(|file| {
                if mode.is_audio()
                    && !options.include_target
                    && extension(file) == mode.target_extension()
                {
                    return false;
                }
                if mode == MediaMode::Video && !options.replace {
                    return !file
                        .file_stem()
                        .and_then(OsStr::to_str)
                        .is_some_and(|stem| stem.to_ascii_lowercase().ends_with("-web"));
                }
                true
            })
            .collect(),
        ..collected
    })
}

fn output_for(mode: MediaMode, source: &Path, options: &Options) -> PathBuf {
    let parent = options
        .output
        .as_deref()
        .unwrap_or_else(|| source.parent().unwrap_or_else(|| Path::new(".")));
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let name = match mode {
        MediaMode::Png => source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        MediaMode::Video if !options.replace => format!("{stem}-web.mp4"),
        _ => format!("{stem}{}", mode.target_extension()),
    };
    parent.join(name)
}

fn plans(mode: MediaMode, options: &Options, files: Vec<PathBuf>) -> ToolResult<Vec<Plan>> {
    let mut plans: Vec<_> = files
        .into_iter()
        .map(|source| {
            let output = output_for(mode, &source, options);
            let same = common::same_path(&source, &output);
            let removes_source = options.output.is_none()
                && !same
                && (matches!(mode, MediaMode::Webp | MediaMode::Avif)
                    || (mode == MediaMode::Video && options.replace)
                    || (mode.is_audio() && options.replace));
            Plan {
                source,
                output,
                output_exists: false,
                overwrites_source: same,
                removes_source,
            }
        })
        .collect();
    common::validate_plans(mode.tool(), &mut plans)?;
    Ok(plans)
}

fn temp_path(output: &Path) -> ToolResult<PathBuf> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| ToolError::new("just", error.to_string()))?;
    let ext = output.extension().and_then(OsStr::to_str).unwrap_or("tmp");
    let suffix = format!(".tmp.{ext}");
    let temporary = tempfile::Builder::new()
        .prefix(".justtools-")
        .suffix(&suffix)
        .tempfile_in(parent)
        .map_err(|error| ToolError::new("just", error.to_string()))?;
    let (_file, path) = temporary
        .keep()
        .map_err(|error| ToolError::new("just", error.error.to_string()))?;
    Ok(path)
}

fn run_process(executable: &Path, args: &[OsString]) -> std::io::Result<Output> {
    let mut command = Command::new(executable);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output()
}

fn compact_failure(output: &Output) -> String {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let lines: Vec<_> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        format!("process exited with {}", output.status)
    } else {
        lines[lines.len().saturating_sub(4)..].join(" | ")
    }
}

fn encoder_available(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .any(|encoder| encoder == name)
}

fn required_ffmpeg_encoder(mode: MediaMode, options: &Options) -> Option<&'static str> {
    match mode {
        MediaMode::Video => Some("libx264"),
        MediaMode::Avif => Some("libaom-av1"),
        MediaMode::Audio => Some("aac"),
        MediaMode::Mp3 => Some("libmp3lame"),
        MediaMode::Wav if options.bits == 24 => Some("pcm_s24le"),
        MediaMode::Wav => Some("pcm_s16le"),
        MediaMode::Png | MediaMode::Webp => None,
    }
}

fn verify_ffmpeg_encoder(mode: MediaMode, options: &Options, executable: &Path) -> ToolResult {
    let Some(required) = required_ffmpeg_encoder(mode, options) else {
        return Ok(());
    };
    let output = run_process(
        executable,
        &[OsString::from("-hide_banner"), OsString::from("-encoders")],
    )
    .map_err(|error| {
        ToolError::new(
            mode.tool(),
            format!("cannot inspect the installed FFmpeg: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(ToolError::new(
            mode.tool(),
            format!(
                "cannot inspect the installed FFmpeg: {}",
                compact_failure(&output)
            ),
        ));
    }
    let listing = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if encoder_available(&listing, required) {
        return Ok(());
    }
    Err(ToolError::new(
        mode.tool(),
        format!(
            "the installed FFmpeg does not include the required `{required}` encoder; install a full FFmpeg build with that encoder, or set FFMPEG_BIN to one"
        ),
    ))
}

fn dependency_executable(mode: MediaMode) -> ToolResult<PathBuf> {
    let variable = match mode {
        MediaMode::Png => "PNGQUANT_BIN",
        MediaMode::Webp => "CWEBP_BIN",
        _ => "FFMPEG_BIN",
    };
    let Some(requested) = std::env::var_os(variable).filter(|value| !value.is_empty()) else {
        return deps::require(mode.tool(), mode.dependency());
    };
    let path = PathBuf::from(&requested);
    if path.is_file() {
        return Ok(path);
    }
    let text = requested.to_str().ok_or_else(|| {
        ToolError::new(
            mode.tool(),
            format!(
                "{variable} points to a missing non-UTF-8 path: {}",
                path.to_string_lossy()
            ),
        )
    })?;
    deps::require(mode.tool(), text)
}

pub(crate) fn animated_png(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut signature = [0_u8; 8];
    if file.read_exact(&mut signature).is_err() || signature != [137, 80, 78, 71, 13, 10, 26, 10] {
        return false;
    }
    loop {
        let mut header = [0_u8; 8];
        if file.read_exact(&mut header).is_err() {
            return false;
        }
        let length = u32::from_be_bytes(header[0..4].try_into().expect("four bytes"));
        let kind = &header[4..8];
        if kind == b"acTL" {
            return true;
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return false;
        }
        if file.seek(SeekFrom::Current(i64::from(length) + 4)).is_err() {
            return false;
        }
    }
}

pub(crate) fn animated_webp(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 12];
    if file.read_exact(&mut header).is_err()
        || &header[0..4] != b"RIFF"
        || &header[8..12] != b"WEBP"
    {
        return false;
    }
    loop {
        let mut chunk = [0_u8; 8];
        if file.read_exact(&mut chunk).is_err() {
            return false;
        }
        if &chunk[0..4] == b"ANIM" || &chunk[0..4] == b"ANMF" {
            return true;
        }
        let length = u32::from_le_bytes(chunk[4..8].try_into().expect("four bytes"));
        let padded = u64::from(length) + u64::from(length % 2);
        let Ok(offset) = i64::try_from(padded) else {
            return false;
        };
        if file.seek(SeekFrom::Current(offset)).is_err() {
            return false;
        }
    }
}

fn ffprobe_for(ffmpeg: &Path) -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("FFPROBE_BIN").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    if let Some(sibling) = ffmpeg.parent().map(|parent| parent.join(name))
        && sibling.is_file()
    {
        return Some(sibling);
    }
    deps::find_executable("ffprobe")
}

fn avif_frame_count(ffprobe: &Path, source: &Path) -> Result<u64, String> {
    let args = [
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-select_streams"),
        OsString::from("v:0"),
        OsString::from("-count_frames"),
        OsString::from("-count_packets"),
        OsString::from("-show_entries"),
        OsString::from("stream=nb_read_frames,nb_read_packets"),
        OsString::from("-of"),
        OsString::from("default=noprint_wrappers=1"),
        source.as_os_str().to_owned(),
    ];
    let output = run_process(ffprobe, &args).map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(compact_failure(&output));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once('=').map(|(_, value)| value.trim()))
        .filter_map(|value| value.parse::<u64>().ok())
        .max()
        .ok_or_else(|| "FFprobe did not report an AVIF frame count".into())
}

fn assert_single_frame_avif(ffmpeg: &Path, source: &Path) -> ToolResult {
    let ffprobe = ffprobe_for(ffmpeg).ok_or_else(|| {
        ToolError::new(
            "justavif",
            "ffprobe is required to verify --include-avif inputs; install the full FFmpeg package or set FFPROBE_BIN",
        )
    })?;
    let frames = avif_frame_count(&ffprobe, source).map_err(|error| {
        ToolError::new(
            "justavif",
            format!(
                "could not verify that {} is a single image: {error}",
                common::display_path(source)
            ),
        )
    })?;
    if frames > 1 {
        return Err(ToolError::new(
            "justavif",
            format!(
                "animated or multi-image AVIF is not supported and was left unchanged: {} ({frames} frames)",
                common::display_path(source)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn multipage_tiff(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 8];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    let little = &header[0..2] == b"II";
    if !little && &header[0..2] != b"MM" {
        return false;
    }
    let read16 = |bytes: [u8; 2]| {
        if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    };
    let read32 = |bytes: [u8; 4]| {
        if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    };
    if read16(header[2..4].try_into().expect("two bytes")) != 42 {
        return false;
    }
    let first = read32(header[4..8].try_into().expect("four bytes"));
    if file.seek(SeekFrom::Start(u64::from(first))).is_err() {
        return false;
    }
    let mut count_bytes = [0_u8; 2];
    if file.read_exact(&mut count_bytes).is_err() {
        return false;
    }
    let count = u64::from(read16(count_bytes));
    let next_offset = u64::from(first)
        .saturating_add(2)
        .saturating_add(count.saturating_mul(12));
    if file.seek(SeekFrom::Start(next_offset)).is_err() {
        return false;
    }
    let mut next = [0_u8; 4];
    file.read_exact(&mut next).is_ok() && read32(next) != 0
}

fn assert_safe(mode: MediaMode, source: &Path) -> ToolResult {
    if matches!(mode, MediaMode::Png | MediaMode::Webp | MediaMode::Avif) && animated_png(source) {
        return Err(ToolError::new(
            mode.tool(),
            format!(
                "animated PNG is not supported and was left unchanged: {}",
                common::display_path(source)
            ),
        ));
    }
    if matches!(mode, MediaMode::Webp | MediaMode::Avif) && animated_webp(source) {
        return Err(ToolError::new(
            mode.tool(),
            format!(
                "animated WebP is not supported and was left unchanged: {}",
                common::display_path(source)
            ),
        ));
    }
    if matches!(mode, MediaMode::Webp | MediaMode::Avif) && multipage_tiff(source) {
        return Err(ToolError::new(
            mode.tool(),
            format!(
                "multi-page TIFF is not supported and was left unchanged: {}",
                common::display_path(source)
            ),
        ));
    }
    Ok(())
}

#[derive(Debug)]
enum Status {
    Done { before: u64, after: u64 },
    Kept(String),
    Warning(String),
    Failed(String),
}

fn avif_transparent(ffmpeg: &Path, source: &Path) -> Result<bool, String> {
    let args = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-i",
        &source.to_string_lossy(),
        "-vf",
        "alphaextract,format=gray,signalstats,metadata=print:file=-",
        "-frames:v",
        "1",
        "-f",
        "null",
        "-",
    ];
    let output = run_process(ffmpeg, &args.iter().map(OsString::from).collect::<Vec<_>>())
        .map_err(|error| error.to_string())?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(value) = text.lines().find_map(|line| {
        line.split_once("lavfi.signalstats.YMIN=")
            .and_then(|(_, value)| value.trim().parse::<f64>().ok())
    }) {
        return Ok(value < 254.5);
    }
    if text.contains("Requested planes not available") {
        return Ok(false);
    }
    if !output.status.success() {
        return Err(compact_failure(&output));
    }
    Ok(false)
}

fn encode(mode: MediaMode, options: &Options, executable: &Path, plan: &Plan) -> Status {
    let before = match fs::metadata(&plan.source) {
        Ok(value) => value.len(),
        Err(error) => return Status::Failed(error.to_string()),
    };
    if mode == MediaMode::Avif {
        match avif_transparent(executable, &plan.source) {
            Ok(true) => {
                return Status::Failed(
                    "transparent image is not supported and was left unchanged".into(),
                );
            }
            Err(error) => return Status::Failed(error),
            _ => {}
        }
    }
    let temporary = match temp_path(&plan.output) {
        Ok(value) => value,
        Err(error) => return Status::Failed(error.message().into()),
    };
    let source = plan.source.as_os_str().to_owned();
    let target = temporary.as_os_str().to_owned();
    let args: Vec<OsString> = match mode {
        MediaMode::Png => ["--quality", &options.quality_range, "--speed", &options.speed.to_string(), "--strip", "--skip-if-larger", "--force", "--output"]
            .into_iter().map(OsString::from).chain([target.clone(), OsString::from("--"), source.clone()]).collect(),
        MediaMode::Webp => ["-quiet", "-q", &options.quality.to_string(), "-m", &options.method.to_string(), "-mt", "-sharp_yuv", "-metadata", "none"]
            .into_iter().map(OsString::from).chain([source.clone(), OsString::from("-o"), target.clone()]).collect(),
        MediaMode::Video => vec!["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"].into_iter().map(OsString::from).chain([source.clone()]).chain([
            "-map", "0:v:0", "-map", "0:a:0?", "-map_metadata", "-1", "-sn", "-dn", "-vf", "scale='min(1280,iw)':'min(720,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2,format=yuv420p", "-c:v", "libx264", "-preset"
        ].into_iter().map(OsString::from)).chain([OsString::from(&options.preset), OsString::from("-crf"), OsString::from(options.crf.to_string()), OsString::from("-c:a"), OsString::from("aac"), OsString::from("-b:a"), OsString::from(&options.audio_bitrate), OsString::from("-movflags"), OsString::from("+faststart"), target.clone()]).collect(),
        MediaMode::Avif => {
            let crf = (63.0 - options.quality as f64 * 0.63).round() as u32;
            vec!["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"].into_iter().map(OsString::from).chain([source.clone()]).chain([
                "-frames:v", "1", "-map_metadata", "-1", "-an", "-sn", "-dn", "-c:v", "libaom-av1", "-still-picture", "1", "-crf"
            ].into_iter().map(OsString::from)).chain([OsString::from(crf.to_string()), OsString::from("-cpu-used"), OsString::from(options.speed.to_string()), OsString::from("-row-mt"), OsString::from("1"), OsString::from("-b:v"), OsString::from("0"), OsString::from("-pix_fmt"), OsString::from("yuv420p"), target.clone()]).collect()
        }
        MediaMode::Audio | MediaMode::Mp3 | MediaMode::Wav => {
            let mut values: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"].into_iter().map(OsString::from).collect();
            values.push(source.clone());
            values.extend(["-map", "0:a:0", "-vn", "-sn", "-dn", "-map_metadata", "-1"].into_iter().map(OsString::from));
            match mode {
                MediaMode::Audio => values.extend([OsString::from("-c:a"), OsString::from("aac"), OsString::from("-profile:a"), OsString::from("aac_low"), OsString::from("-b:a"), OsString::from(&options.audio_bitrate), OsString::from("-ar"), OsString::from(options.sample_rate.to_string()), OsString::from("-movflags"), OsString::from("+faststart")]),
                MediaMode::Mp3 => values.extend([OsString::from("-c:a"), OsString::from("libmp3lame"), OsString::from("-q:a"), OsString::from(options.quality.to_string()), OsString::from("-ar"), OsString::from(options.sample_rate.to_string())]),
                MediaMode::Wav => values.extend([OsString::from("-c:a"), OsString::from(if options.bits == 24 { "pcm_s24le" } else { "pcm_s16le" }), OsString::from("-ar"), OsString::from(options.sample_rate.to_string()), OsString::from("-ac"), OsString::from("2")]),
                _ => unreachable!(),
            }
            values.push(target.clone());
            values
        }
    };
    let output = match run_process(executable, &args) {
        Ok(value) => value,
        Err(error) => {
            fs::remove_file(&temporary).ok();
            return Status::Failed(error.to_string());
        }
    };
    let code = output.status.code();
    if mode == MediaMode::Png && matches!(code, Some(98 | 99)) {
        fs::remove_file(&temporary).ok();
        return Status::Kept(if code == Some(98) {
            format!("cannot meet quality floor {}", options.quality_range)
        } else {
            "optimized PNG is not smaller".into()
        });
    }
    if !output.status.success() {
        fs::remove_file(&temporary).ok();
        return Status::Failed(compact_failure(&output));
    }
    let after = match fs::metadata(&temporary) {
        Ok(value) if value.len() > 0 => value.len(),
        Ok(_) => {
            fs::remove_file(&temporary).ok();
            return Status::Failed("encoder produced an empty output".into());
        }
        Err(error) => return Status::Failed(format!("encoder produced no output: {error}")),
    };
    if matches!(mode, MediaMode::Png | MediaMode::Webp | MediaMode::Avif)
        && options.output.is_none()
        && after >= before
    {
        fs::remove_file(&temporary).ok();
        return Status::Kept(format!(
            "{} is not smaller",
            mode.target_extension()[1..].to_ascii_uppercase()
        ));
    }
    if let Err(error) = common::atomic_install(mode.tool(), &temporary, &plan.output) {
        fs::remove_file(&temporary).ok();
        return Status::Failed(error.message().into());
    }
    if plan.removes_source
        && let Err(error) = fs::remove_file(&plan.source)
    {
        return Status::Warning(format!(
            "output was created, but source could not be removed: {error}"
        ));
    }
    Status::Done { before, after }
}

fn print_dry_run(mode: MediaMode, plans: &[Plan]) {
    println!("{}: dry run — {} file(s)", mode.tool(), plans.len());
    for plan in plans.iter().take(100) {
        let suffix = if plan.removes_source {
            " (then remove source)"
        } else if plan.overwrites_source {
            " (replace atomically)"
        } else {
            ""
        };
        println!(
            "  {} -> {}{suffix}",
            common::display_path(&plan.source),
            common::display_path(&plan.output)
        );
    }
    if plans.len() > 100 {
        println!("  ... and {} more", plans.len() - 100);
    }
}

fn duration(value: Duration) -> String {
    if value.as_secs() < 1 {
        format!("{} ms", value.as_millis())
    } else {
        format!("{:.1} s", value.as_secs_f64())
    }
}

pub fn run(mode: MediaMode, args: Vec<OsString>) -> ToolResult {
    common::init_signals();
    let options = parse(mode, args)?;
    if options.help {
        println!("{}", help(mode));
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
    let collected = collect(mode, &options, &inputs)?;
    for warning in &collected.warnings {
        eprintln!("{}: {warning}", mode.tool());
    }
    if collected.files.is_empty() {
        return Err(ToolError::new(mode.tool(), "no supported files found"));
    }
    for file in &collected.files {
        assert_safe(mode, file)?;
    }
    let plans = plans(mode, &options, collected.files)?;
    if options.dry_run {
        print_dry_run(mode, &plans);
        return Ok(());
    }
    let destructive = plans
        .iter()
        .any(|plan| plan.output_exists || plan.overwrites_source || plan.removes_source);
    if collected.used_directory
        && !piped
        && destructive
        && !options.yes
        && !common::confirm(
            mode.tool(),
            &format!(
                "{}: process {} file(s) and replace existing data",
                mode.tool(),
                plans.len()
            ),
        )?
    {
        return Err(ToolError::cancelled(mode.tool()));
    }
    let executable = dependency_executable(mode)?;
    verify_ffmpeg_encoder(mode, &options, &executable)?;
    if mode == MediaMode::Avif {
        for plan in &plans {
            if extension(&plan.source) == ".avif" {
                assert_single_frame_avif(&executable, &plan.source)?;
            }
        }
    }
    let started = Instant::now();
    println!(
        "{}: {} file(s), {} job(s)",
        mode.tool(),
        plans.len(),
        options.jobs
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()
        .map_err(|error| ToolError::new(mode.tool(), error.to_string()))?;
    let results: Vec<Status> = pool.install(|| {
        plans
            .par_iter()
            .map(|plan| {
                if common::interrupted() {
                    Status::Failed("interrupted".into())
                } else {
                    encode(mode, &options, &executable, plan)
                }
            })
            .collect()
    });
    let mut done = 0;
    let mut kept = 0;
    let mut failed = 0;
    let mut warnings = 0;
    for (plan, status) in plans.iter().zip(&results) {
        let mapping = if common::same_path(&plan.source, &plan.output) {
            common::display_path(&plan.source)
        } else {
            format!(
                "{} -> {}",
                common::display_path(&plan.source),
                common::display_path(&plan.output)
            )
        };
        match status {
            Status::Done { before, after } => {
                done += 1;
                println!(
                    "  done    {mapping}  {} -> {}",
                    common::format_bytes(*before),
                    common::format_bytes(*after)
                );
            }
            Status::Kept(reason) => {
                kept += 1;
                println!(
                    "  kept    {} ({reason})",
                    common::display_path(&plan.source)
                );
            }
            Status::Warning(error) => {
                warnings += 1;
                eprintln!("  warning {mapping}: {error}");
            }
            Status::Failed(error) => {
                failed += 1;
                eprintln!("  failed  {}: {error}", common::display_path(&plan.source));
            }
        }
    }
    println!(
        "{}: {done} done{}{}{} in {}",
        mode.tool(),
        if kept > 0 {
            format!(", {kept} kept")
        } else {
            String::new()
        },
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
        duration(started.elapsed())
    );
    if failed > 0 || warnings > 0 {
        Err(ToolError::with_code(
            mode.tool(),
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
    fn mode_defaults_are_bounded() {
        for mode in [
            MediaMode::Png,
            MediaMode::Webp,
            MediaMode::Video,
            MediaMode::Avif,
            MediaMode::Audio,
        ] {
            assert!((1..=8).contains(&mode.default_jobs()));
        }
    }

    #[test]
    fn rejects_invalid_quality() {
        let error = parse(MediaMode::Png, vec!["--quality".into(), "90-20".into()]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn encoder_listing_matches_whole_names() {
        let listing = " V....D libx264 H.264 / AVC\n A..... aac AAC\n";
        assert!(encoder_available(listing, "libx264"));
        assert!(encoder_available(listing, "aac"));
        assert!(!encoder_available(listing, "x264"));
    }

    #[test]
    fn encoder_output_path_stays_reserved() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("result.webp");
        let temporary = temp_path(&output).unwrap();
        assert!(temporary.is_file());
        assert_eq!(temporary.extension(), Some(OsStr::new("webp")));
    }
}
