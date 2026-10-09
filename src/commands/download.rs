//! Fetching media URLs with yt-dlp before the normal encode pipeline runs.
//!
//! A downloaded file is only ever an intermediate: it lands in a temporary
//! workspace, is converted with the calling tool's own settings, and is removed
//! with that workspace. yt-dlp refreshes itself at most once every twelve hours.

use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;

use crate::deps;
use crate::error::{ToolError, ToolResult};

const UPDATE_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
const STAMP_NAME: &str = "yt-dlp-update";
/// yt-dlp never writes under a title. A caption full of emoji, or one long
/// enough to pass a path limit once ".f137.mp4.part" follows it, fails inside
/// yt-dlp or FFmpeg before anything could be renamed.
const OUTPUT_TEMPLATE: &str = "%(autonumber)03d.%(ext)s";
/// One line per finished file: its path, title, and id, each as JSON.
const NAMES_TEMPLATE: &str = "after_move:%(filepath)j\t%(title)j\t%(id)j";

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

fn stamp_path(executable: &Path, suffix: &str) -> Option<PathBuf> {
    let defaults = crate::preferences::path().ok()?;
    let path = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let hash = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    Some(
        defaults
            .parent()?
            .join(format!("{STAMP_NAME}-{}-{suffix}", &hash[..16])),
    )
}

fn dependency_lock(executable: &Path) -> Result<fs::File, String> {
    let path = stamp_path(executable, "lock").ok_or("cannot locate yt-dlp update state")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        if crate::common::shutdown_requested()
            || (crate::progress::current().is_some() && crate::common::interrupted())
        {
            return Err("cancelled".into());
        }
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(error) => return Err(error.to_string()),
        }
        if crate::progress::current().is_some() && crate::common::interrupted() {
            return Err("cancelled".into());
        }
        if started.elapsed() >= Duration::from_secs(120) {
            return Err("yt-dlp is busy in another operation".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn due(now: SystemTime, stamp: &Path) -> bool {
    due_after(now, stamp, UPDATE_INTERVAL)
}

fn due_after(now: SystemTime, stamp: &Path, interval: Duration) -> bool {
    let Ok(text) = fs::read_to_string(stamp) else {
        return true;
    };
    let Ok(seconds) = text.trim().parse::<u64>() else {
        return true;
    };
    let Some(recorded) = UNIX_EPOCH.checked_add(Duration::from_secs(seconds)) else {
        return true;
    };
    match now.duration_since(recorded) {
        Ok(elapsed) => elapsed >= interval,
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

pub(crate) fn hide_console(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Give cancellation ownership of the tool and its Git/media helpers.
        command.process_group(0);
    }
    #[cfg(windows)]
    if crate::common::background() {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let _ = command;
}

fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // Spawned tools have their own process group, so remote helpers cannot
        // keep our output pipes open after the parent tool has been cancelled.
        unsafe {
            kill(-(child.id() as i32), 9);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn wait_child(
    child: &mut Child,
    timeout: Option<Duration>,
    cancellable: bool,
) -> Result<ExitStatus, String> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Err(error) => {
                stop_child(child);
                return Err(error.to_string());
            }
            _ => {}
        }
        if crate::common::shutdown_requested() || (cancellable && crate::common::interrupted()) {
            stop_child(child);
            return Err("cancelled".into());
        }
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            stop_child(child);
            return Err("process timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn progress_line(line: &str, reporter: &crate::progress::Reporter) {
    if line.starts_with("JUSTTOOLS_POSTPROCESS") {
        reporter.report(
            "Finishing media…",
            "Combining or processing downloaded streams.",
            None,
        );
    } else if let Some(values) = line.strip_prefix("JUSTTOOLS_PROGRESS ") {
        let numbers: Vec<Option<f64>> = values
            .split_whitespace()
            .take(4)
            .map(|n| n.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0))
            .collect();
        let number = |i| numbers.get(i).copied().flatten();
        let percent = number(0)
            .zip(number(1).filter(|n| *n > 0.0))
            .map(|(done, total)| (done / total * 100.0).clamp(0.0, 100.0) as u32);
        let mut detail = number(0)
            .map(|n| format!("{} downloaded", crate::common::format_bytes(n as u64)))
            .unwrap_or_default();
        if let Some(speed) = number(2) {
            detail.push_str(&format!(
                " · {}/s",
                crate::common::format_bytes(speed as u64)
            ));
        }
        if let Some(eta) = number(3) {
            detail.push_str(&format!(
                " · {}:{:02} left",
                eta as u64 / 60,
                eta as u64 % 60
            ));
        }
        reporter.report("Downloading media…", detail, percent);
    }
}

fn read_output(
    mut reader: impl Read,
    reporter: Option<crate::progress::Reporter>,
    on_line: fn(&str, &crate::progress::Reporter),
) -> String {
    let mut tail = Vec::new();
    let mut line = Vec::new();
    let mut buffer = [0; 4096];
    while let Ok(count) = reader.read(&mut buffer) {
        if count == 0 {
            break;
        }
        tail.extend_from_slice(&buffer[..count]);
        if tail.len() > 8192 {
            tail.drain(..tail.len() - 8192);
        }
        if let Some(reporter) = &reporter {
            for &byte in &buffer[..count] {
                if byte == b'\n' || byte == b'\r' {
                    on_line(&String::from_utf8_lossy(&line), reporter);
                    line.clear();
                } else if line.len() < 65536 {
                    line.push(byte);
                }
            }
        }
    }
    String::from_utf8_lossy(&tail).into_owned()
}

fn capture(
    command: &mut Command,
    timeout: Option<Duration>,
    cancellable: bool,
) -> Result<(ExitStatus, String), String> {
    capture_with_progress(command, timeout, cancellable, progress_line)
}

/// Drain bounded output while supporting cancellation and tool-specific progress.
pub(crate) fn capture_with_progress(
    command: &mut Command,
    timeout: Option<Duration>,
    cancellable: bool,
    on_line: fn(&str, &crate::progress::Reporter),
) -> Result<(ExitStatus, String), String> {
    if crate::common::shutdown_requested() || (cancellable && crate::common::interrupted()) {
        return Err("cancelled".into());
    }
    hide_console(command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("missing process output pipe")?;
    let stderr = child.stderr.take().ok_or("missing process error pipe")?;
    let reporter = crate::progress::current();
    let other = reporter.clone();
    let output = std::thread::spawn(move || read_output(stdout, reporter, on_line));
    let error = std::thread::spawn(move || read_output(stderr, other, on_line));
    let status = wait_child(&mut child, timeout, cancellable);
    let output = output.join().unwrap_or_default();
    let error = error.join().unwrap_or_default();
    status.map(|status| (status, format!("{output}\n{error}")))
}

/// Run `yt-dlp --update` unless it already ran within the last twelve hours.
///
/// Updating is best-effort: a package-managed yt-dlp refuses to replace itself,
/// and an offline machine cannot reach the release feed. Either way the
/// download still proceeds with the installed version.
pub fn update(tool: &str, executable: &Path) {
    let Ok(_lease) = dependency_lock(executable) else {
        return;
    };
    let now = SystemTime::now();
    let stamp = stamp_path(executable, "success");
    let attempt = stamp_path(executable, "attempt");
    if stamp.as_ref().is_some_and(|p| !due(now, p))
        || attempt
            .as_ref()
            .is_some_and(|p| !due_after(now, p, Duration::from_secs(15 * 60)))
    {
        return;
    }
    crate::progress::stage("Checking for a yt-dlp update…");
    println!("{tool}: checking for a newer yt-dlp");
    let mut command = Command::new(executable);
    command.args(["--ignore-config", "--update", "--socket-timeout", "15"]);
    hide_console(&mut command);
    #[cfg(windows)]
    let cancellable = !crate::common::background() || crate::progress::current().is_some();
    #[cfg(not(windows))]
    let cancellable = true;
    let result = capture(&mut command, Some(Duration::from_secs(90)), cancellable);
    let success = match result {
        Ok((status, text)) => {
            let summary = last_line(&text).unwrap_or("yt-dlp reported nothing");
            if status.success() {
                println!("{tool}: {summary}");
            } else {
                eprintln!(
                    "{tool}: yt-dlp could not update itself: {summary}; retrying in 15 minutes"
                );
            }
            status.success()
        }
        Err(error) => {
            eprintln!("{tool}: cannot update yt-dlp: {error}");
            false
        }
    };
    let finished = SystemTime::now();
    if let Some(path) = &attempt {
        record(path, finished);
    }
    if success && let Some(path) = &stamp {
        record(path, finished);
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
    args.extend([
        OsString::from("--ffmpeg-location"),
        ffmpeg.as_os_str().to_owned(),
    ]);
    args.extend([OsString::from("-P"), directory.as_os_str().to_owned()]);
    args.extend([OsString::from("-o"), OsString::from(OUTPUT_TEMPLATE)]);
    // The file to print to is itself a template, so a literal % is doubled.
    let mut names = OsString::new();
    for (index, piece) in names_file(directory)
        .to_string_lossy()
        .split('%')
        .enumerate()
    {
        if index > 0 {
            names.push("%%");
        }
        names.push(piece);
    }
    args.extend([
        OsString::from("--print-to-file"),
        OsString::from(NAMES_TEMPLATE),
        names,
    ]);
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

/// Where yt-dlp records what each file in a download folder is called. It
/// sits beside the folder so it is never mistaken for a download.
fn names_file(directory: &Path) -> PathBuf {
    directory.with_extension("names")
}

/// An id is only a tiebreaker, and a generic page's id can be a whole address.
const MAX_ID_CHARS: usize = 24;

/// A title without the parts that only count its audience. Facebook titles
/// read "470 reactions · 15 shares | the caption | the uploader".
fn without_counts(title: &str) -> String {
    const COUNTED: [&str; 12] = [
        "reaction",
        "reactions",
        "share",
        "shares",
        "view",
        "views",
        "like",
        "likes",
        "comment",
        "comments",
        "play",
        "plays",
    ];
    let counts = |part: &str| {
        part.split('·').all(|count| {
            let mut words = count.split_whitespace();
            let number = words.next().is_some_and(|number| {
                number.starts_with(|digit: char| digit.is_ascii_digit())
                    && number
                        .chars()
                        .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | 'K' | 'M' | 'B'))
            });
            let counted = words
                .next()
                .is_some_and(|word| COUNTED.contains(&word.to_ascii_lowercase().as_str()));
            number && counted && words.next().is_none()
        })
    };
    let parts: Vec<&str> = title.split(" | ").filter(|part| !counts(part)).collect();
    parts.join(" | ")
}

/// The tidied title and id yt-dlp recorded for each downloaded file, by its
/// file name. Either is empty when yt-dlp had nothing usable for it.
fn recorded_names(names: &Path) -> Vec<(OsString, String, String)> {
    let text = fs::read_to_string(names).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t').map(|field| {
                serde_json::from_str::<String>(field)
                    .map(|value| crate::common::tidy_stem(&without_counts(&value)))
                    .unwrap_or_default()
            });
            let file = line.split('\t').next()?;
            let file = PathBuf::from(serde_json::from_str::<String>(file).ok()?);
            fields.next();
            let title = fields.next().unwrap_or_default();
            let id = fields.next().unwrap_or_default();
            let id = id.chars().take(MAX_ID_CHARS).collect::<String>();
            Some((file.file_name()?.to_owned(), title, id.trim().to_owned()))
        })
        .collect()
}

/// A downloaded file, the URL it came from, and what to call it.
///
/// `stem` is the plain title and is unique within one `fetch`. `id` is the
/// site's own short id, empty when there is none, for a caller whose output
/// may replace an existing file and so must not confuse two same-titled videos.
#[derive(Clone, Debug)]
pub struct Fetched {
    pub url: String,
    pub file: PathBuf,
    pub stem: String,
    pub id: String,
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
        let mut command = Command::new(executable);
        command.args(arguments(kind, ffmpeg, &directory, url, playlist));
        hide_console(&mut command);
        crate::progress::stage("Waiting for yt-dlp...");
        let _lease = dependency_lock(executable).map_err(|e| ToolError::new(tool, e))?;
        crate::progress::stage("Reading media information...");
        let (status, detail) = if crate::progress::current().is_some() {
            let args = arguments(kind, ffmpeg, &directory, url, playlist);
            command = Command::new(executable);
            command.args(["--newline", "--progress", "--progress-delta", "0.2", "--progress-template",
                "download:JUSTTOOLS_PROGRESS %(progress.downloaded_bytes)s %(progress.total_bytes,progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s",
                "--progress-template", "postprocess:JUSTTOOLS_POSTPROCESS"]);
            command.args(args);
            hide_console(&mut command);
            capture(&mut command, None, true).map_err(|e| ToolError::new(tool, e))?
        } else {
            let mut child = command
                .spawn()
                .map_err(|e| ToolError::new(tool, format!("cannot run yt-dlp: {e}")))?;
            (
                wait_child(&mut child, None, true).map_err(|e| ToolError::new(tool, e))?,
                String::new(),
            )
        };
        if !status.success() {
            let reason = last_line(&detail).unwrap_or("see yt-dlp output above");
            return Err(ToolError::new(
                tool,
                format!("yt-dlp could not download {url}: {reason}"),
            ));
        }
        crate::progress::stage("Saving downloaded files...");
        let files = downloaded_files(&directory);
        if files.is_empty() {
            return Err(ToolError::new(
                tool,
                format!("yt-dlp reported success but saved nothing for {url}"),
            ));
        }
        let recorded = recorded_names(&names_file(&directory));
        for file in files {
            let (title, id) = recorded
                .iter()
                .find(|(name, _, _)| file.file_name() == Some(name.as_os_str()))
                .map(|(_, title, id)| (title.clone(), id.clone()))
                .unwrap_or_default();
            // Without a title the id names the file, then whatever yt-dlp
            // called it.
            let stem = [title, id.clone()]
                .into_iter()
                .chain(
                    file.file_stem()
                        .map(|stem| crate::common::tidy_stem(&stem.to_string_lossy())),
                )
                .find(|stem| !stem.is_empty())
                .unwrap_or_else(|| "download".into());
            // Two entries of a playlist may share a title.
            let stem = (1_u32..)
                .map(|number| match number {
                    1 => stem.clone(),
                    number => format!("{stem} ({number})"),
                })
                .find(|candidate| {
                    !downloaded.iter().any(|known: &Fetched| {
                        known.stem.to_lowercase() == candidate.to_lowercase()
                    })
                })
                .expect("an unused number always exists");
            downloaded.push(Fetched {
                url: url.clone(),
                file,
                stem,
                id,
            });
        }
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
        fs::write(&stamp, u64::MAX.to_string()).unwrap();
        assert!(due(now, &stamp));
        record(&stamp, now + Duration::from_secs(60 * 60 * 24));
        assert!(due(now, &stamp));
    }

    #[test]
    fn failed_update_retry_is_fifteen_minutes_not_twelve_hours() {
        let temp = tempfile::tempdir().unwrap();
        let success = temp.path().join("success");
        let attempt = temp.path().join("attempt");
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        record(&attempt, now);
        assert!(due(now, &success));
        assert!(!due_after(
            now + Duration::from_secs(899),
            &attempt,
            Duration::from_secs(900)
        ));
        assert!(due_after(
            now + Duration::from_secs(900),
            &attempt,
            Duration::from_secs(900)
        ));
        record(&success, now);
        assert!(!due(now + Duration::from_secs(900), &success));
    }

    #[test]
    fn downloads_use_a_fixed_name_and_record_their_titles_beside_the_folder() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("100% real").join("0");
        let rendered: Vec<_> = arguments(
            Kind::Original,
            Path::new("ffmpeg"),
            &directory,
            "https://example.test/watch?v=1",
            false,
        )
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
        assert!(rendered.contains(&"%(autonumber)03d.%(ext)s".to_owned()));
        assert!(!rendered.iter().any(|value| value.contains("%(title)s")));
        let names = &rendered[rendered.iter().position(|v| v == NAMES_TEMPLATE).unwrap() + 1];
        assert!(names.contains("100%% real") && names.ends_with("0.names"));

        let names = temp.path().join("0.names");
        let folder = serde_json::to_string(&directory.join("001.mp4")).unwrap();
        fs::write(
            &names,
            [
                format!("{folder}\t\"POV: it worked \\ud83d\\ude2d #fyp\"\t\"abc123\""),
                "\"002.mp4\"\t\"\\ud83d\\ude2d\\ud83d\\ude2d\"\t\"xyz-789\"".into(),
                "\"003.mp4\"\tNA\tNA".into(),
                "not json at all".into(),
            ]
            .join("\n"),
        )
        .unwrap();
        let long_id = format!("\"004.mp4\"\t\"Clip\"\t\"{}\"", "z".repeat(90));
        fs::write(
            &names,
            format!("{}\n{long_id}\n", fs::read_to_string(&names).unwrap()),
        )
        .unwrap();
        let name =
            |file: &str, title: &str, id: &str| (file.into(), title.to_owned(), id.to_owned());
        assert_eq!(
            recorded_names(&names),
            [
                name("001.mp4", "POV it worked", "abc123"),
                name("002.mp4", "", "xyz-789"),
                name("003.mp4", "", ""),
                name("004.mp4", "Clip", &"z".repeat(MAX_ID_CHARS)),
            ]
        );
        assert!(recorded_names(&temp.path().join("missing.names")).is_empty());
        assert_eq!(
            without_counts("470 reactions · 15 shares | Imagine this | Some Uploader"),
            "Imagine this | Some Uploader"
        );
        assert_eq!(without_counts("1.2K views | Clip"), "Clip");
        assert_eq!(
            without_counts("3 shares of stock | 2 views on it"),
            "3 shares of stock | 2 views on it"
        );
        assert_eq!(without_counts("12 views"), "");
    }

    #[cfg(windows)]
    #[test]
    fn streamed_download_progress_handles_unknown_totals_and_postprocessing() {
        let operation = crate::progress::Operation::begin();
        let reporter = operation.reporter.clone();
        let data = b"site status\nJUSTTOOLS_PROGRESS 512 1024 128 4\n";
        read_output(&data[..], Some(reporter.clone()), progress_line);
        let state = reporter.snapshot();
        assert_eq!(state.percent, Some(50));
        assert!(state.detail.contains("0:04 left"));
        progress_line("JUSTTOOLS_PROGRESS 512 NA NA NA", &reporter);
        assert_eq!(reporter.snapshot().percent, None);
        progress_line("JUSTTOOLS_PROGRESS NaN 0 inf -4", &reporter);
        assert_eq!(reporter.snapshot().percent, None);
        progress_line("JUSTTOOLS_POSTPROCESS", &reporter);
        assert!(reporter.snapshot().stage.contains("Finishing"));
        assert_eq!(
            read_output(&vec![b'x'; 100_000][..], None, progress_line).len(),
            8192
        );
    }

    #[cfg(windows)]
    #[test]
    fn captured_process_cancellation_stops_the_child_and_drains_output() {
        let operation = crate::progress::Operation::begin();
        let mut command = Command::new("ping.exe");
        command.args(["-n", "20", "127.0.0.1"]);
        let started = Instant::now();
        let result = capture_with_progress(&mut command, None, true, |line, reporter| {
            if !line.trim().is_empty() {
                reporter.cancel();
            }
        });
        assert_eq!(result.unwrap_err(), "cancelled");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(operation.reporter.snapshot().cancelled);
    }

    #[cfg(windows)]
    #[test]
    fn update_timeout_kills_its_child_and_drains_pipes() {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "ping -n 20 127.0.0.1 >nul"]);
        let started = Instant::now();
        let result = capture(&mut command, Some(Duration::from_millis(150)), false);
        assert!(result.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(10));
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
