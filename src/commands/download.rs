//! Fetching media URLs with yt-dlp before the normal encode pipeline runs.
//!
//! A downloaded file is only ever an intermediate: it lands in a temporary
//! workspace, is converted with the calling tool's own settings, and is removed
//! with that workspace. yt-dlp refreshes itself at most once every twelve hours.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;

use crate::deps;
use crate::error::{ToolError, ToolResult};

const UPDATE_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
const STAMP_NAME: &str = "yt-dlp-update";
const OUTPUT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

/// What the calling tool needs out of a URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// `max_height` is the tallest stream worth fetching; `None` takes the best.
    Video {
        max_height: Option<u32>,
    },
    Audio,
    /// The best version of whatever the page offers, kept as downloaded.
    Original,
}

impl Kind {
    fn format(self) -> String {
        match self {
            // The encode scales down to its own target, so a taller source
            // than that only costs time.
            Self::Video {
                max_height: Some(height),
            } => format!("bv*[height<={height}]+ba/b[height<={height}]/bv*+ba/b"),
            Self::Video { max_height: None } => "bv*+ba/b".into(),
            Self::Audio => "ba/b".into(),
            Self::Original => "bv*+ba/b".into(),
        }
    }
}

/// Whether an input names a downloadable address rather than a path.
pub fn is_url(value: &OsStr) -> bool {
    value.to_str().is_some_and(|text| {
        let lower = text.trim().to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    })
}

/// Resolve yt-dlp, honoring `YTDLP_BIN`.
pub fn executable(tool: &str) -> ToolResult<PathBuf> {
    deps::require_override(tool, "YTDLP_BIN", "yt-dlp")
}

fn stamp_path() -> Option<PathBuf> {
    let defaults = crate::preferences::path().ok()?;
    Some(defaults.parent()?.join(STAMP_NAME))
}

fn due(now: SystemTime, stamp: &Path) -> bool {
    let Ok(text) = fs::read_to_string(stamp) else {
        return true;
    };
    let Ok(seconds) = text.trim().parse::<u64>() else {
        return true;
    };
    match now.duration_since(UNIX_EPOCH + Duration::from_secs(seconds)) {
        Ok(elapsed) => elapsed >= UPDATE_INTERVAL,
        // A stamp from the future only costs one extra check; recording the
        // current time immediately afterwards repairs the clock skew.
        Err(_) => true,
    }
}

fn record(stamp: &Path, now: SystemTime) {
    let Ok(since) = now.duration_since(UNIX_EPOCH) else {
        return;
    };
    if let Some(parent) = stamp.parent() {
        fs::create_dir_all(parent).ok();
    }
    if let Ok(mut file) = AtomicWriteFile::open(stamp)
        && file
            .write_all(since.as_secs().to_string().as_bytes())
            .is_ok()
    {
        file.commit().ok();
    }
}

fn last_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).rfind(|line| !line.is_empty())
}

/// Run `yt-dlp --update` unless it already ran within the last twelve hours.
///
/// Updating is best-effort: a package-managed yt-dlp refuses to replace itself,
/// and an offline machine cannot reach the release feed. Either way the
/// download still proceeds with the installed version.
pub fn update(tool: &str, executable: &Path) {
    let now = SystemTime::now();
    let stamp = stamp_path();
    if let Some(path) = &stamp
        && !due(now, path)
    {
        return;
    }
    println!("{tool}: checking for a newer yt-dlp");
    match Command::new(executable).arg("--update").output() {
        Ok(output) => {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let summary = last_line(&text)
                .unwrap_or("yt-dlp reported nothing")
                .to_owned();
            if output.status.success() {
                println!("{tool}: {summary}");
            } else {
                eprintln!("{tool}: yt-dlp could not update itself: {summary}");
            }
        }
        Err(error) => eprintln!("{tool}: cannot run yt-dlp --update: {error}"),
    }
    if let Some(path) = &stamp {
        record(path, now);
    }
}

/// Keep an installed yt-dlp current even when this run downloads nothing.
///
/// Sites change faster than yt-dlp releases reach package managers, so every
/// run of a tool that can download checks whether twelve hours have passed.
/// A machine without yt-dlp is left alone; it is fetched when first needed.
pub fn refresh(tool: &str) {
    if let Some(executable) = deps::installed_yt_dlp() {
        update(tool, &executable);
    }
}

fn arguments(
    kind: Kind,
    ffmpeg: &Path,
    directory: &Path,
    url: &str,
    playlist: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    // A personal yt-dlp configuration could redirect the output or re-encode
    // behind our back, which headless runs must stay free of.
    args.push(OsString::from("--ignore-config"));
    args.push(OsString::from(if playlist {
        "--yes-playlist"
    } else {
        "--no-playlist"
    }));
    args.extend([OsString::from("-f"), OsString::from(kind.format())]);
    if matches!(kind, Kind::Video { .. }) {
        args.extend([
            OsString::from("--merge-output-format"),
            OsString::from("mkv"),
        ]);
    }
    if kind == Kind::Original {
        // Nothing re-encodes this file afterwards: take the highest
        // resolution, and at that resolution the streams that play
        // everywhere, in an MP4.
        args.extend(
            [
                "-S",
                "res,vcodec:h264,ext:mp4:m4a",
                "--merge-output-format",
                "mp4",
            ]
            .map(OsString::from),
        );
    }
    args.extend([OsString::from("--trim-filenames"), OsString::from("120")]);
    args.extend([
        OsString::from("--ffmpeg-location"),
        ffmpeg.as_os_str().to_owned(),
    ]);
    args.extend([OsString::from("-P"), directory.as_os_str().to_owned()]);
    args.extend([OsString::from("-o"), OsString::from(OUTPUT_TEMPLATE)]);
    args.extend([OsString::from("--"), OsString::from(url)]);
    args
}

fn downloaded_files(directory: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = walkdir::WalkDir::new(directory)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    files
}

/// A downloaded file and the URL it came from.
#[derive(Clone, Debug)]
pub struct Fetched {
    pub url: String,
    pub file: PathBuf,
}

/// Download every URL into its own folder below `workspace`.
///
/// yt-dlp keeps the terminal so its progress stays visible; a failure is
/// reported after it has already explained itself.
pub fn fetch(
    tool: &str,
    kind: Kind,
    executable: &Path,
    ffmpeg: &Path,
    urls: &[String],
    playlist: bool,
    workspace: &Path,
) -> ToolResult<Vec<Fetched>> {
    let mut downloaded = Vec::new();
    for (index, url) in urls.iter().enumerate() {
        let directory = workspace.join(index.to_string());
        fs::create_dir_all(&directory).map_err(|error| {
            ToolError::new(tool, format!("cannot prepare a download folder: {error}"))
        })?;
        println!("{tool}: downloading {url}");
        let status = Command::new(executable)
            .args(arguments(kind, ffmpeg, &directory, url, playlist))
            .status()
            .map_err(|error| ToolError::new(tool, format!("cannot run yt-dlp: {error}")))?;
        if !status.success() {
            return Err(ToolError::new(
                tool,
                format!("yt-dlp could not download {url}"),
            ));
        }
        let files = downloaded_files(&directory);
        if files.is_empty() {
            return Err(ToolError::new(
                tool,
                format!("yt-dlp reported success but saved nothing for {url}"),
            ));
        }
        downloaded.extend(files.into_iter().map(|file| Fetched {
            url: url.clone(),
            file,
        }));
    }
    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_addresses_are_treated_as_urls() {
        assert!(is_url(OsStr::new("https://example.test/clip")));
        assert!(is_url(OsStr::new("HTTP://EXAMPLE.TEST/clip")));
        assert!(is_url(OsStr::new("  https://example.test/clip  ")));
        assert!(!is_url(OsStr::new("clip.mp4")));
        assert!(!is_url(OsStr::new("ftp://example.test/clip")));
        assert!(!is_url(OsStr::new("C:/videos/https_clip.mp4")));
    }

    #[test]
    fn updates_run_once_per_twelve_hours() {
        let temp = tempfile::tempdir().unwrap();
        let stamp = temp.path().join(STAMP_NAME);
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert!(due(now, &stamp), "a missing stamp must update");
        record(&stamp, now);
        assert!(!due(now + Duration::from_secs(11 * 60 * 60), &stamp));
        assert!(due(now + UPDATE_INTERVAL, &stamp));
        // A damaged or future stamp updates once and is then rewritten.
        fs::write(&stamp, "not a timestamp").unwrap();
        assert!(due(now, &stamp));
        record(&stamp, now + Duration::from_secs(60 * 60 * 24));
        assert!(due(now, &stamp));
    }

    #[test]
    fn video_downloads_are_capped_and_merged_while_audio_skips_video() {
        let video = arguments(
            Kind::Video {
                max_height: Some(1080),
            },
            Path::new("/opt/ffmpeg"),
            Path::new("/work/0"),
            "https://example.test/watch?v=1",
            false,
        );
        let rendered: Vec<_> = video
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert!(rendered.contains(&"--ignore-config".to_owned()));
        assert!(rendered.contains(&"--no-playlist".to_owned()));
        assert!(rendered.contains(&"bv*[height<=1080]+ba/b[height<=1080]/bv*+ba/b".to_owned()));
        assert!(rendered.contains(&"mkv".to_owned()));
        assert_eq!(rendered.last().unwrap(), "https://example.test/watch?v=1");
        assert_eq!(rendered[rendered.len() - 2], "--");

        let audio = arguments(
            Kind::Audio,
            Path::new("/opt/ffmpeg"),
            Path::new("/work/0"),
            "https://example.test/watch?v=1",
            true,
        );
        let rendered: Vec<_> = audio
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert!(rendered.contains(&"ba/b".to_owned()));
        assert_eq!(
            Kind::Video {
                max_height: Some(2160)
            }
            .format(),
            "bv*[height<=2160]+ba/b[height<=2160]/bv*+ba/b"
        );
        assert_eq!(Kind::Video { max_height: None }.format(), "bv*+ba/b");

        let original: Vec<_> = arguments(
            Kind::Original,
            Path::new("/opt/ffmpeg"),
            Path::new("/work/0"),
            "https://example.test/watch?v=1",
            false,
        )
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
        assert!(original.contains(&"res,vcodec:h264,ext:mp4:m4a".to_owned()));
        assert!(original.contains(&"mp4".to_owned()));
        assert!(!original.contains(&"mkv".to_owned()));
        assert!(rendered.contains(&"--yes-playlist".to_owned()));
        assert!(!rendered.contains(&"--merge-output-format".to_owned()));
    }
}
