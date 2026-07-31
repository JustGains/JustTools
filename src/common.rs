use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{ToolError, ToolResult};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

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
    INTERRUPTED.load(Ordering::SeqCst)
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

fn friendly_path_text(path: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
