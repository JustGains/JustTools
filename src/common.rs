use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{ToolError, ToolResult};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static BACKGROUND: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static BACKGROUND_STOPPING: AtomicBool = AtomicBool::new(false);

pub(crate) fn shutdown_requested() -> bool {
    #[cfg(windows)]
    {
        BACKGROUND_STOPPING.load(Ordering::SeqCst)
    }
    #[cfg(not(windows))]
    {
        false
    }
}
#[cfg(windows)]
pub(crate) fn stop_background() {
    BACKGROUND_STOPPING.store(true, Ordering::SeqCst);
}

#[cfg(windows)]
pub(crate) fn set_background() {
    BACKGROUND.store(true, Ordering::SeqCst);
}

#[cfg(windows)]
pub(crate) fn background() -> bool {
    BACKGROUND.load(Ordering::SeqCst)
}

#[derive(Clone, Debug)]
pub struct InputOptions<'a> {
    pub extensions: &'a [&'a str],
    pub recursive: bool,
    pub exclude_directory: Option<&'a Path>,
}

#[derive(Debug)]
pub struct CollectedPaths {
    pub files: Vec<PathBuf>,
    pub used_directory: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub source: PathBuf,
    pub output: PathBuf,
    pub output_exists: bool,
    pub overwrites_source: bool,
    pub removes_source: bool,
}

pub fn init_signals() {
    let _ = ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::SeqCst));
}

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst) || crate::progress::cancelled()
}

#[cfg(windows)]
pub(crate) fn set_interrupted(value: bool) {
    INTERRUPTED.store(value, Ordering::SeqCst);
}

pub fn os_to_string(tool: &str, value: &OsStr, label: &str) -> ToolResult<String> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| ToolError::usage(tool, format!("{label} must be valid UTF-8")))
}

pub fn option_value(
    tool: &str,
    args: &[OsString],
    index: &mut usize,
    option: &str,
) -> ToolResult<String> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| ToolError::usage(tool, format!("{option} needs a value")))?;
    os_to_string(tool, value, option)
}

pub fn integer(
    tool: &str,
    value: &str,
    label: &str,
    minimum: u32,
    maximum: u32,
) -> ToolResult<u32> {
    value
        .parse::<u32>()
        .ok()
        .filter(|number| *number >= minimum && *number <= maximum)
        .ok_or_else(|| {
            ToolError::usage(
                tool,
                format!("{label} must be an integer from {minimum} to {maximum}"),
            )
        })
}

pub fn read_stdin() -> ToolResult<String> {
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| ToolError::new("just", format!("could not read stdin: {error}")))?;
    Ok(text)
}

pub fn stdin_is_terminal() -> bool {
    io::stdin().is_terminal()
}
pub fn stdout_is_terminal() -> bool {
    io::stdout().is_terminal()
}

pub fn parse_input_lines(text: &str) -> Vec<OsString> {
    if text.contains('\0') {
        text.split('\0')
            .filter(|line| !line.is_empty())
            .map(OsString::from)
            .collect()
    } else {
        text.lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(OsString::from)
            .collect()
    }
}

fn lexical_absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn resolve_nearest(path: &Path) -> PathBuf {
    let absolute = lexical_absolute(path);
    if let Ok(resolved) = absolute.canonicalize() {
        return resolved;
    }
    let mut ancestor = absolute.clone();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name().map(OsStr::to_owned) else {
            return absolute;
        };
        missing.push(name);
        if !ancestor.pop() {
            return absolute;
        }
    }
    let Ok(mut resolved) = ancestor.canonicalize() else {
        return absolute;
    };
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    resolved
}

pub(crate) fn friendly_path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return rest.to_owned();
        }
    }
    text.into_owned()
}

fn normalized(path: &Path) -> String {
    let resolved = resolve_nearest(path);
    let text = friendly_path_text(&resolved).replace('\\', "/");
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    normalized(left) == normalized(right)
}

pub fn display_path(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    #[cfg(windows)]
    let text = {
        let absolute = friendly_path_text(&absolute);
        std::env::current_dir()
            .ok()
            .map(|cwd| friendly_path_text(&cwd))
            .and_then(|cwd| {
                let cwd = cwd.trim_end_matches(['/', '\\']);
                if absolute.eq_ignore_ascii_case(cwd) {
                    return Some(".".to_owned());
                }
                let prefix = format!("{cwd}\\");
                absolute
                    .get(prefix.len()..)
                    .filter(|_| absolute[..prefix.len()].eq_ignore_ascii_case(&prefix))
                    .map(str::to_owned)
            })
            .unwrap_or(absolute)
    };
    #[cfg(not(windows))]
    let text = {
        let shown = std::env::current_dir()
            .ok()
            .and_then(|cwd| absolute.strip_prefix(cwd).ok().map(Path::to_path_buf))
            .unwrap_or(absolute);
        shown.to_string_lossy().into_owned()
    };
    if text.contains(char::is_whitespace) {
        format!("\"{text}\"")
    } else {
        text
    }
}

pub fn collect_paths(
    tool: &str,
    inputs: &[OsString],
    options: &InputOptions<'_>,
) -> ToolResult<CollectedPaths> {
    let extensions: HashSet<String> = options
        .extensions
        .iter()
        .map(|extension| extension.to_ascii_lowercase())
        .collect();
    let excluded = options.exclude_directory.map(normalized);
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut used_directory = false;

    fn scan(
        tool: &str,
        directory: &Path,
        options: &InputOptions<'_>,
        extensions: &HashSet<String>,
        excluded: &Option<String>,
        output: &mut Vec<PathBuf>,
    ) -> ToolResult<usize> {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .map_err(|error| {
                ToolError::new(
                    tool,
                    format!("cannot read {}: {error}", display_path(directory)),
                )
            })?
            .collect::<Result<_, _>>()
            .map_err(|error| {
                ToolError::new(
                    tool,
                    format!("cannot read {}: {error}", display_path(directory)),
                )
            })?;
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());
        let mut found = 0;
        for entry in entries {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| ToolError::new(tool, error.to_string()))?;
            if file_type.is_dir() {
                if options.recursive
                    && excluded
                        .as_ref()
                        .is_none_or(|target| normalized(&path) != *target)
                {
                    found += scan(tool, &path, options, extensions, excluded, output)?;
                }
            } else if file_type.is_file() {
                let extension = path
                    .extension()
                    .and_then(OsStr::to_str)
                    .map(|value| format!(".{}", value.to_ascii_lowercase()));
                if extension
                    .as_ref()
                    .is_some_and(|extension| extensions.contains(extension))
                {
                    output.push(path.canonicalize().unwrap_or(path));
                    found += 1;
                }
            }
        }
        Ok(found)
    }

    for input in inputs {
        let path = PathBuf::from(input);
        let absolute = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        let metadata = fs::metadata(&absolute).map_err(|_| {
            ToolError::new(tool, format!("path not found: {}", input.to_string_lossy()))
        })?;
        if metadata.is_dir() {
            used_directory = true;
            if scan(tool, &absolute, options, &extensions, &excluded, &mut files)? == 0 {
                warnings.push(format!("no supported files in {}", display_path(&absolute)));
            }
        } else if metadata.is_file() {
            let extension = absolute
                .extension()
                .and_then(OsStr::to_str)
                .map(|value| format!(".{}", value.to_ascii_lowercase()));
            if !extension
                .as_ref()
                .is_some_and(|extension| extensions.contains(extension))
            {
                return Err(ToolError::new(
                    tool,
                    format!("unsupported file: {}", display_path(&absolute)),
                ));
            }
            files.push(absolute.canonicalize().unwrap_or(absolute));
        } else {
            return Err(ToolError::new(
                tool,
                format!("not a file or folder: {}", input.to_string_lossy()),
            ));
        }
    }

    let mut seen = HashSet::new();
    files.retain(|file| seen.insert(normalized(file)));
    Ok(CollectedPaths {
        files,
        used_directory,
        warnings,
    })
}

pub fn validate_plans(tool: &str, plans: &mut [Plan]) -> ToolResult {
    let mut outputs: HashMap<String, PathBuf> = HashMap::new();
    let sources: HashSet<String> = plans.iter().map(|plan| normalized(&plan.source)).collect();
    for plan in plans {
        if plan.output.is_dir() {
            return Err(ToolError::new(
                tool,
                format!("output path is a folder: {}", display_path(&plan.output)),
            ));
        }
        plan.output_exists = plan.output.exists();
        let key = normalized(&plan.output);
        if let Some(prior) = outputs.insert(key.clone(), plan.source.clone())
            && !same_path(&prior, &plan.source)
        {
            return Err(ToolError::new(
                tool,
                format!(
                    "two inputs would write {}: {} and {}",
                    display_path(&plan.output),
                    display_path(&prior),
                    display_path(&plan.source)
                ),
            ));
        }
        if sources.contains(&key) && !same_path(&plan.source, &plan.output) {
            return Err(ToolError::new(
                tool,
                format!(
                    "{} is both an output and another input",
                    display_path(&plan.output)
                ),
            ));
        }
    }
    Ok(())
}

pub fn confirm(tool: &str, question: &str) -> ToolResult<bool> {
    if !stdin_is_terminal() || !stdout_is_terminal() {
        return Err(ToolError::new(
            tool,
            "confirmation requires a terminal; re-run with --yes",
        ));
    }
    eprint!("{question} [y/N] ");
    io::stderr().flush().ok();
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| ToolError::new(tool, error.to_string()))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Publish a completed directory atomically, including when an empty directory
/// or dangling link appeared at its destination after planning.
pub fn rename_directory_noclobber(temporary: &Path, output: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let path = |value: &Path| {
            CString::new(value.as_os_str().as_bytes())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
        };
        let source = path(temporary)?;
        let target = path(output)?;
        #[cfg(target_os = "linux")]
        unsafe extern "C" {
            fn renameat2(
                olddirfd: i32,
                oldpath: *const std::ffi::c_char,
                newdirfd: i32,
                newpath: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        #[cfg(target_os = "macos")]
        unsafe extern "C" {
            fn renamex_np(
                from: *const std::ffi::c_char,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        // Both APIs refuse every existing target. The paths remain alive for
        // the call; flags are RENAME_NOREPLACE (Linux) / RENAME_EXCL (macOS).
        #[cfg(target_os = "linux")]
        let result = unsafe { renameat2(-100, source.as_ptr(), -100, target.as_ptr(), 1) };
        #[cfg(target_os = "macos")]
        let result = unsafe { renamex_np(source.as_ptr(), target.as_ptr(), 4) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Windows rename never replaces an existing directory. Check other
        // entry types too, so files and dangling links also stay intact.
        match fs::symlink_metadata(output) {
            Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(temporary, output)
    }
}

pub fn atomic_install(tool: &str, temporary: &Path, output: &Path) -> ToolResult {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| ToolError::new(tool, error.to_string()))?;
    }
    if !output.exists() {
        fs::rename(temporary, output).map_err(|error| {
            ToolError::new(
                tool,
                format!("could not install {}: {error}", display_path(output)),
            )
        })?;
        return Ok(());
    }
    let backup = output.with_extension(format!(
        "{}.justtools-backup-{}",
        output.extension().and_then(OsStr::to_str).unwrap_or(""),
        std::process::id()
    ));
    fs::rename(output, &backup).map_err(|error| {
        ToolError::new(
            tool,
            format!("could not prepare {}: {error}", display_path(output)),
        )
    })?;
    if let Err(error) = fs::rename(temporary, output) {
        if let Err(rollback) = fs::rename(&backup, output) {
            return Err(ToolError::new(
                tool,
                format!(
                    "could not install {}: {error}; rollback also failed: {rollback}. The previous file remains at {}",
                    display_path(output),
                    display_path(&backup)
                ),
            ));
        }
        return Err(ToolError::new(
            tool,
            format!("could not install {}: {error}", display_path(output)),
        ));
    }
    fs::remove_file(&backup).map_err(|error| {
        ToolError::new(
            tool,
            format!(
                "output installed but backup remains at {}: {error}",
                display_path(&backup)
            ),
        )
    })?;
    Ok(())
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let units = ["KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", value, units[unit.saturating_sub(1)])
}

const MAX_STEM_CHARS: usize = 80;
/// Leaves room under the 255-byte limit of most file systems for an
/// extension, a " (2)" suffix, and a temporary suffix while it is written.
const MAX_STEM_BYTES: usize = 180;

fn stem_character(character: char) -> char {
    match character {
        '\u{2018}' | '\u{2019}' => '\'',
        '\u{2013}' | '\u{2014}' => '-',
        '-' | '_' | '.' | ',' | '\'' | '(' | ')' | '&' | '+' => character,
        character if character.is_alphanumeric() => character,
        // Accents and vowel signs that complete the letter before them.
        '\u{0300}'..='\u{036F}' | '\u{0900}'..='\u{0DFF}' | '\u{0E00}'..='\u{0E7F}' => character,
        _ => ' ',
    }
}

fn trim_stem(stem: &str) -> &str {
    stem.trim_matches([' ', '.', '-', ','])
}

/// A title or a server-supplied name as a short, plain file stem.
///
/// Letters and digits of any script are kept with a little punctuation.
/// Emoji, symbols, links, and a caption's hashtags are dropped, and a long
/// name is cut at a word near eighty characters. The result is empty when
/// nothing usable is left, so the caller chooses the fallback.
pub fn tidy_stem(raw: &str) -> String {
    let words: Vec<&str> = raw
        .split_whitespace()
        .filter(|word| {
            let lower = word.to_ascii_lowercase();
            !(lower.starts_with("http://")
                || lower.starts_with("https://")
                || lower.starts_with("www."))
        })
        .collect();
    let join = |hashtags: bool| {
        let mut joined = String::new();
        for word in &words {
            let hashtag = word
                .strip_prefix('#')
                .and_then(|tag| tag.chars().next())
                .is_some_and(char::is_alphabetic);
            if hashtag && !hashtags {
                continue;
            }
            for character in word.chars().map(stem_character).chain([' ']) {
                let repeated = matches!(character, ' ' | '.') && joined.ends_with(character);
                if !repeated {
                    joined.push(character);
                }
            }
        }
        trim_stem(&joined).to_owned()
    };
    // Hashtags are noise beside a caption, but they are the name when they
    // are all there is.
    let mut stem = join(false);
    if stem.is_empty() {
        stem = join(true);
    }
    let mut end = stem
        .char_indices()
        .nth(MAX_STEM_CHARS)
        .map_or(stem.len(), |(index, _)| index)
        .min(MAX_STEM_BYTES);
    if end < stem.len() {
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        // End on a whole word when that keeps most of the name.
        if !stem[end..].starts_with(' ')
            && let Some(space) = stem[..end].rfind(' ')
            && space >= end / 2
        {
            end = space;
        }
        stem = trim_stem(&stem[..end]).to_owned();
    }
    // Windows reserves these device names whatever follows the first dot.
    let device = stem.split('.').next().unwrap_or_default().trim_end();
    let reserved = ["con", "prn", "aux", "nul"].contains(&device.to_ascii_lowercase().as_str())
        || (device.len() == 4
            && device.get(..3).is_some_and(|port| {
                port.eq_ignore_ascii_case("com") || port.eq_ignore_ascii_case("lpt")
            })
            && device.ends_with(|digit: char| digit.is_ascii_digit()));
    if reserved {
        stem.insert(0, '_');
    }
    stem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_publication_is_atomic_and_preserves_existing_entries() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("staged");
        let target = directory.path().join("published");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("kept"), "checkout").unwrap();
        fs::create_dir(&target).unwrap();
        assert!(rename_directory_noclobber(&source, &target).is_err());
        assert!(source.join("kept").is_file());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        fs::remove_dir(&target).unwrap();
        fs::write(&target, "existing file").unwrap();
        assert!(rename_directory_noclobber(&source, &target).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "existing file");
        fs::remove_file(&target).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(directory.path().join("missing"), &target).unwrap();
            assert!(rename_directory_noclobber(&source, &target).is_err());
            assert!(
                fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(&target).unwrap();
        }
        rename_directory_noclobber(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(target.join("kept")).unwrap(), "checkout");
    }

    #[test]
    fn stems_are_short_plain_and_safe_on_every_platform() {
        assert_eq!(tidy_stem("Me at the zoo"), "Me at the zoo");
        assert_eq!(
            tidy_stem("POV: you finally did it 😭😭 #fyp #gym https://t.co/abc"),
            "POV you finally did it"
        );
        assert_eq!(tidy_stem("🔥 #fyp #Leg-Day"), "fyp Leg-Day");
        assert_eq!(tidy_stem("Track #3 — live"), "Track 3 - live");
        assert_eq!(
            tidy_stem("  what?!\r\n<now>  \"then\"...  "),
            "what now then"
        );
        assert_eq!(tidy_stem("Wait... what"), "Wait. what");
        assert_eq!(tidy_stem("abé1"), "abé1");
        assert_eq!(tidy_stem("../../etc/passwd"), "etc passwd");
        assert_eq!(
            tidy_stem("AC/DC: Rock’n’Roll (Live)"),
            "AC DC Rock'n'Roll (Live)"
        );
        assert_eq!(tidy_stem("naïve café 東京の夜"), "naïve café 東京の夜");
        assert_eq!(tidy_stem("IMG_1234.final-v2"), "IMG_1234.final-v2");
        assert_eq!(tidy_stem("😭😭 🔥"), "");
        assert_eq!(tidy_stem("con"), "_con");
        assert_eq!(tidy_stem("COM1.backup"), "_COM1.backup");
        assert_eq!(tidy_stem("console"), "console");

        let long = tidy_stem(&"every word counts ".repeat(20));
        assert!(long.chars().count() <= MAX_STEM_CHARS, "{long}");
        assert!(long.ends_with("counts") || long.ends_with("word") || long.ends_with("every"));
        let unbroken = tidy_stem(&"x".repeat(400));
        assert_eq!(unbroken.chars().count(), MAX_STEM_CHARS);
        let wide = tidy_stem(&"東".repeat(400));
        assert!(wide.len() <= MAX_STEM_BYTES && wide.chars().all(|c| c == '東'));
    }

    #[test]
    fn parses_lines_and_nulls() {
        assert_eq!(
            parse_input_lines("a\r\nb\n"),
            vec![OsString::from("a"), OsString::from("b")]
        );
        assert_eq!(
            parse_input_lines("a\0b\0"),
            vec![OsString::from("a"), OsString::from("b")]
        );
    }

    #[test]
    fn byte_format_is_compact() {
        assert_eq!(format_bytes(12), "12 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
    }

    #[test]
    fn lexical_paths_do_not_escape_the_filesystem_root() {
        let root = Path::new(std::path::MAIN_SEPARATOR_STR);
        let normalized = lexical_absolute(&root.join("..").join("outside"));
        assert_eq!(normalized.file_name(), Some(OsStr::new("outside")));
        assert!(
            !normalized
                .components()
                .any(|component| component == Component::ParentDir)
        );
    }

    #[cfg(windows)]
    #[test]
    fn display_text_removes_windows_verbatim_prefixes() {
        assert_eq!(
            friendly_path_text(Path::new(r"\\?\C:\work\file.txt")),
            r"C:\work\file.txt"
        );
        assert_eq!(
            friendly_path_text(Path::new(r"\\?\UNC\server\share\file.txt")),
            r"\\server\share\file.txt"
        );
    }
}
