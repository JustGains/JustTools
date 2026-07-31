use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use clap::error::ErrorKind;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

thread_local! {
    /// Optional argv supplied by the root multicall binary.
    static CLI_ARGS: RefCell<Option<Vec<OsString>>> = const { RefCell::new(None) };
}

struct CliArgsGuard(Option<Vec<OsString>>);

impl Drop for CliArgsGuard {
    fn drop(&mut self) {
        CLI_ARGS.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

#[derive(Debug)]
pub struct CollectedFiles {
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub used_directory: bool,
}

pub fn read_stdin() -> Result<String> {
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .context("could not read stdin")?;
    Ok(text)
}

pub fn stdin_is_terminal() -> bool {
    io::stdin().is_terminal()
}

pub fn stdout_is_terminal() -> bool {
    io::stdout().is_terminal()
}

/// Parses a command while preserving clap's successful help/version behavior.
pub fn parse_cli<T: Parser>() -> Result<Option<T>> {
    let supplied = CLI_ARGS.with(|slot| slot.borrow().clone());
    let parsed = match supplied {
        Some(args) => T::try_parse_from(args),
        None => T::try_parse(),
    };
    match parsed {
        Ok(options) => Ok(Some(options)),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.print().context("could not print command help")?;
            Ok(None)
        }
        Err(error) => Err(anyhow!(error.to_string())),
    }
}

/// Runs a command with an explicit argv, allowing the same implementation to
/// serve both standalone binaries and the root `just` multicall executable.
pub fn with_cli_args<R>(tool: &str, args: Vec<OsString>, run: impl FnOnce() -> R) -> R {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(OsString::from(tool));
    argv.extend(args);
    let previous = CLI_ARGS.with(|slot| slot.replace(Some(argv)));
    let _guard = CliArgsGuard(previous);
    run()
}

pub fn collect_files(
    raw_inputs: &[PathBuf],
    extension: &str,
    recursive: bool,
    excluded_directory: Option<&Path>,
) -> Result<CollectedFiles> {
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut used_directory = false;
    let extension = extension.trim_start_matches('.');
    let excluded = excluded_directory.map(absolute_lexical).transpose()?;

    for raw in raw_inputs {
        let input = absolute_lexical(raw)?;
        if !input.exists() {
            warnings.push(format!("{}: not found", display_path(&input)));
            continue;
        }
        if input.is_file() {
            if has_extension(&input, extension) {
                files.push(input);
            } else {
                warnings.push(format!("{}: not a .{extension} file", display_path(&input)));
            }
            continue;
        }
        if !input.is_dir() {
            warnings.push(format!("{}: not a file or folder", display_path(&input)));
            continue;
        }

        used_directory = true;
        let max_depth = if recursive { usize::MAX } else { 1 };
        for entry in WalkDir::new(&input)
            .min_depth(1)
            .max_depth(max_depth)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    warnings.push(format!("{}: {error}", display_path(&input)));
                    continue;
                }
            };
            if entry.file_type().is_dir()
                && excluded
                    .as_ref()
                    .is_some_and(|path| same_path(entry.path(), path))
            {
                continue;
            }
            if entry.file_type().is_file() && has_extension(entry.path(), extension) {
                if excluded
                    .as_ref()
                    .is_some_and(|path| is_within(entry.path(), path))
                {
                    continue;
                }
                files.push(absolute_lexical(entry.path())?);
            }
        }
    }

    let mut seen = HashSet::new();
    files.retain(|file| seen.insert(path_key(file)));
    Ok(CollectedFiles {
        files,
        warnings,
        used_directory,
    })
}

pub fn validate_unique_outputs(plans: &[(PathBuf, PathBuf)]) -> Result<()> {
    let sources: HashSet<_> = plans.iter().map(|(source, _)| path_key(source)).collect();
    let mut outputs = HashSet::new();
    for (source, output) in plans {
        let key = path_key(output);
        if !outputs.insert(key.clone()) {
            bail!("multiple inputs resolve to {}", display_path(output));
        }
        if !same_path(source, output) && sources.contains(&key) {
            bail!(
                "an output would overwrite another input: {}",
                display_path(output)
            );
        }
    }
    Ok(())
}

pub fn confirm(prompt: &str) -> Result<bool> {
    if !stdin_is_terminal() || !stdout_is_terminal() {
        bail!("confirmation requires a terminal; re-run with --yes");
    }
    eprint!("{prompt}? [y/N] ");
    io::stderr().flush().ok();
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("could not read confirmation")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

pub fn confirm_replacement(path: &Path, yes: bool) -> Result<()> {
    if path.exists() && !yes && !confirm(&format!("replace {}", display_path(path)))? {
        bail!("cancelled");
    }
    Ok(())
}

/// Writes beside the destination, flushes it, and atomically replaces the destination.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_with(path, |file| file.write_all(bytes).map_err(Into::into))
}

pub fn atomic_write_with(path: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("could not create {}", display_path(parent)))?;
    let existing_permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", display_path(path)));
        }
    };
    let mut temporary = NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "could not create a temporary file in {}",
            display_path(parent)
        )
    })?;
    write(temporary.as_file_mut())?;
    if let Some(permissions) = existing_permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .with_context(|| {
                format!("could not preserve permissions for {}", display_path(path))
            })?;
    }
    temporary
        .as_file_mut()
        .sync_all()
        .context("could not flush temporary output")?;
    let temporary_path = temporary.into_temp_path();
    replace_file(&temporary_path, path)
        .with_context(|| format!("could not install {}", display_path(path)))?;
    // The path has moved; prevent TempPath from attempting cleanup.
    temporary_path.keep().ok();
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for this call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn absolute_lexical(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("could not determine current folder")?
            .join(path)
    };
    Ok(normalize_lexically(&absolute))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                );
                if can_pop {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn display_path(path: &Path) -> String {
    let current = std::env::current_dir().ok();
    if let Some(relative) = current.as_ref().and_then(|cwd| path.strip_prefix(cwd).ok())
        && !relative.as_os_str().is_empty()
    {
        return relative.display().to_string();
    }
    path.display().to_string()
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn parse_piped_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

fn is_within(path: &Path, directory: &Path) -> bool {
    let path = path_key(path);
    let directory = path_key(directory);
    path == directory
        || path
            .strip_prefix(&directory)
            .is_some_and(|rest| rest.starts_with(std::path::MAIN_SEPARATOR))
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn path_key(path: &Path) -> String {
    let key = normalize_lexically(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

pub fn file_name(path: &Path) -> Result<OsString> {
    path.file_name()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("{} has no file name", display_path(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_readably() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn parses_nonempty_piped_paths() {
        assert_eq!(
            parse_piped_paths("a\n\n b \r\n"),
            [PathBuf::from("a"), PathBuf::from("b")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replacement_preserves_existing_unix_mode() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("output.json");
        fs::write(&output, b"old").unwrap();
        fs::set_permissions(&output, fs::Permissions::from_mode(0o640)).unwrap();

        atomic_write(&output, b"new").unwrap();

        assert_eq!(
            fs::metadata(output).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn lexical_normalization_never_pops_a_unix_root() {
        assert_eq!(
            normalize_lexically(Path::new("/../../tmp")),
            Path::new("/tmp")
        );
    }

    #[cfg(windows)]
    #[test]
    fn lexical_normalization_never_pops_a_windows_root() {
        assert_eq!(
            normalize_lexically(Path::new(r"C:\..\..\tmp")),
            Path::new(r"C:\tmp")
        );
        assert_eq!(
            normalize_lexically(Path::new(r"\\server\share\..\tmp")),
            Path::new(r"\\server\share\tmp")
        );
    }
}
