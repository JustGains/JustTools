use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::common;
use crate::deps;
use crate::error::{ToolError, ToolResult};

const HELP: &str = r#"justzip — Archive files and folders, honoring Git ignore rules in working trees.

Usage:
  justzip [options] [file-or-folder ...]

In direct/headless mode, `.` archives the current folder. One input produces
<name>.zip (including a file's extension); several inputs produce archive.zip
in the current directory. A single folder contributes its contents; multiple
inputs keep their file/folder names. Sources are kept and existing ZIP outputs
are replaced atomically. Conflicting archive names are rejected before writing.

Ordinary folders include all contents recursively, including empty folders,
except .git metadata. Git folders use Git's tracked and unignored file list,
so nested .gitignore files, .git/info/exclude, and global ignores apply.
Explicitly selected files are included even if ignored. Git is needed only
for Git folders. Symlinks are stored without following them; unsafe links fail.

Run bare to open the interactive launcher; explicit arguments bypass the UI.

Options:
  -o, --output PATH       Output ZIP file, or an existing output directory
  -c, --compression MODE fast, balanced, or smallest (default: smallest)
  -n, --dry-run           Show what would be archived without writing
  -h, --help              Show this help"#;

#[derive(Clone, Copy, Debug)]
enum Compression {
    Fast,
    Balanced,
    Smallest,
}

impl Compression {
    fn parse(value: &str) -> ToolResult<Self> {
        match value.to_ascii_lowercase().as_str() {
            "fast" => Ok(Self::Fast),
            "balanced" => Ok(Self::Balanced),
            "smallest" => Ok(Self::Smallest),
            _ => Err(ToolError::usage(
                "justzip",
                "compression must be fast, balanced, or smallest",
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Smallest => "smallest",
        }
    }

    fn level(self) -> i64 {
        match self {
            Self::Fast => 1,
            Self::Balanced => 6,
            Self::Smallest => 9,
        }
    }
}

struct Options {
    sources: Vec<PathBuf>,
    output: Option<PathBuf>,
    compression: Compression,
    dry_run: bool,
    help: bool,
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        sources: Vec::new(),
        output: None,
        compression: Compression::Smallest,
        dry_run: false,
        help: false,
    };
    let mut positional = false;
    let mut index = 0;
    while index < args.len() {
        if !positional && args[index] == std::ffi::OsStr::new("--") {
            positional = true;
            index += 1;
            continue;
        }
        let Some(original) = args[index].to_str() else {
            options.sources.push(PathBuf::from(&args[index]));
            index += 1;
            continue;
        };
        let (option, inline) = original
            .split_once('=')
            .filter(|_| original.starts_with("--"))
            .map_or((original, None), |(key, value)| {
                (key, Some(value.to_owned()))
            });
        let value = |index: &mut usize| -> ToolResult<String> {
            if let Some(value) = &inline {
                if value.is_empty() {
                    return Err(ToolError::usage(
                        "justzip",
                        format!("{option} needs a value"),
                    ));
                }
                Ok(value.clone())
            } else {
                common::option_value("justzip", &args, index, option)
            }
        };
        let path_value = |index: &mut usize| -> ToolResult<OsString> {
            if let Some(value) = &inline {
                if value.is_empty() {
                    return Err(ToolError::usage(
                        "justzip",
                        format!("{option} needs a value"),
                    ));
                }
                return Ok(OsString::from(value));
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| ToolError::usage("justzip", format!("{option} needs a value")))
        };
        if !positional {
            match option {
                "-h" | "--help" => options.help = true,
                "-n" | "--dry-run" => options.dry_run = true,
                "-o" | "--output" => options.output = Some(PathBuf::from(path_value(&mut index)?)),
                "-c" | "--compression" => {
                    options.compression = Compression::parse(&value(&mut index)?)?
                }
                _ if original.starts_with('-') => {
                    return Err(ToolError::usage(
                        "justzip",
                        format!("unknown option: {original}"),
                    ));
                }
                _ => {
                    options.sources.push(PathBuf::from(&args[index]));
                }
            }
        } else {
            options.sources.push(PathBuf::from(&args[index]));
        }
        index += 1;
    }
    Ok(options)
}

fn resolved_source(path: &Path) -> ToolResult<PathBuf> {
    let absolute = lexical_absolute(path)?;
    let metadata = fs::symlink_metadata(&absolute).map_err(|error| {
        ToolError::new(
            "justzip",
            format!("cannot inspect {}: {error}", path.display()),
        )
    })?;
    if !metadata.is_dir() && !metadata.is_file() && !metadata.file_type().is_symlink() {
        return Err(ToolError::new(
            "justzip",
            format!("not a file or folder: {}", path.display()),
        ));
    }
    let Some(name) = absolute.file_name() else {
        return Err(ToolError::new(
            "justzip",
            "refusing to archive a filesystem root",
        ));
    };
    if metadata.file_type().is_symlink() {
        // Resolve the parent, never a selected link's target.
        absolute
            .parent()
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map(|parent| parent.join(name))
    } else {
        absolute.canonicalize()
    }
    .map_err(|error| {
        ToolError::new(
            "justzip",
            format!("cannot resolve {}: {error}", path.display()),
        )
    })
}

fn lexical_absolute(path: &Path) -> ToolResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| ToolError::new("justzip", error.to_string()))?
            .join(path)
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
    Ok(normalized)
}

fn resolved_output(path: &Path) -> ToolResult<PathBuf> {
    let normalized = lexical_absolute(path)?;
    if fs::symlink_metadata(&normalized).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ToolError::new(
            "justzip",
            format!("refusing symlink output: {}", normalized.display()),
        ));
    }
    let file_name = normalized
        .file_name()
        .ok_or_else(|| ToolError::new("justzip", "output must name a ZIP file"))?
        .to_owned();
    let mut ancestor = normalized
        .parent()
        .ok_or_else(|| ToolError::new("justzip", "output has no parent folder"))?
        .to_path_buf();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| ToolError::new("justzip", "cannot resolve output folder"))?
            .to_owned();
        missing.push(name);
        if !ancestor.pop() {
            return Err(ToolError::new("justzip", "cannot resolve output folder"));
        }
    }
    let mut resolved = ancestor.canonicalize().map_err(|error| {
        ToolError::new("justzip", format!("cannot resolve output folder: {error}"))
    })?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    resolved.push(file_name);
    Ok(resolved)
}

pub(crate) fn destination(sources: &[PathBuf], argument: Option<&Path>) -> ToolResult<PathBuf> {
    let archive_name = match sources {
        [source] => format!("{}.zip", source_name(source)?),
        _ => "archive.zip".to_owned(),
    };
    let current =
        std::env::current_dir().map_err(|error| ToolError::new("justzip", error.to_string()))?;
    let output = match argument {
        None => current.join(archive_name),
        Some(value) => {
            let candidate = if value.is_absolute() {
                value.to_path_buf()
            } else {
                current.join(value)
            };
            let looks_like_directory = value.as_os_str().to_string_lossy().ends_with(['/', '\\']);
            if candidate.is_dir() || looks_like_directory {
                candidate.join(archive_name)
            } else {
                candidate
            }
        }
    };
    resolved_output(&output)
}

fn git_file_list(git: &Path, source: &Path) -> ToolResult<Vec<String>> {
    let output = Command::new(git)
        .arg("-C")
        .arg(source)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|error| ToolError::new("justzip", format!("could not start Git: {error}")))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(ToolError::new(
            "justzip",
            if error.is_empty() {
                "the folder is not inside a Git working tree".into()
            } else {
                error
            },
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| ToolError::new("justzip", "Git returned a non-UTF-8 path"))
        .map(|text| {
            text.split('\0')
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()
        })
}

struct Entry {
    full_path: PathBuf,
    name: String,
    kind: EntryKind,
}

enum EntryKind {
    File,
    Symlink(PathBuf),
    Directory,
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':'])
        && !Path::new(value).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn safe_symlink_target(entry_name: &str, target: &Path) -> bool {
    let Some(text) = target.to_str() else {
        return false;
    };
    // ZIP link targets use forward slashes on every platform. Never turn a
    // literal Unix backslash or a Windows drive name into an escaping link.
    if text.contains(':') || (!cfg!(windows) && text.contains('\\')) {
        return false;
    }
    if target.is_absolute() {
        return false;
    }
    let mut depth = Path::new(entry_name)
        .parent()
        .map_or(0, |parent| parent.components().count());
    for component in target.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

fn collect_entries(
    source: &Path,
    destination: &Path,
    names: Vec<String>,
    git: bool,
) -> ToolResult<(Vec<Entry>, usize, usize)> {
    let relative_output = destination
        .strip_prefix(source)
        .ok()
        .map(|value| value.to_string_lossy().replace('\\', "/"));
    let mut entries = Vec::new();
    let mut skipped_gitlinks = 0;
    let mut skipped_missing = 0;
    let mut seen = std::collections::HashSet::new();
    for name in names {
        let normalized = if cfg!(windows) {
            name.replace('\\', "/")
        } else {
            name
        };
        if !safe_relative(&normalized) {
            return Err(ToolError::new(
                "justzip",
                format!("unsafe archive path: {normalized}"),
            ));
        }
        if relative_output.as_deref().is_some_and(|value| {
            if cfg!(windows) {
                value.eq_ignore_ascii_case(&normalized)
            } else {
                value == normalized
            }
        }) || !seen.insert(normalized.clone())
        {
            continue;
        }
        let full_path = source.join(Path::new(&normalized));
        let metadata = match fs::symlink_metadata(&full_path) {
            Ok(metadata) => metadata,
            Err(error) if git && error.kind() == std::io::ErrorKind::NotFound => {
                skipped_missing += 1;
                continue;
            }
            Err(error) => {
                return Err(ToolError::new(
                    "justzip",
                    format!("cannot inspect {}: {error}", full_path.display()),
                ));
            }
        };
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&full_path).map_err(|error| {
                ToolError::new(
                    "justzip",
                    format!("cannot read symlink {}: {error}", full_path.display()),
                )
            })?;
            if !safe_symlink_target(&normalized, &target) {
                return Err(ToolError::new(
                    "justzip",
                    format!(
                        "refusing unsafe symlink {} -> {}",
                        normalized,
                        target.display()
                    ),
                ));
            }
            entries.push(Entry {
                full_path,
                name: normalized,
                kind: EntryKind::Symlink(target),
            });
        } else if metadata.is_file() {
            let canonical = full_path.canonicalize().map_err(|error| {
                ToolError::new(
                    "justzip",
                    format!("cannot resolve {}: {error}", full_path.display()),
                )
            })?;
            if !canonical.starts_with(source) {
                return Err(ToolError::new(
                    "justzip",
                    format!("refusing file outside source folder: {normalized}"),
                ));
            }
            entries.push(Entry {
                full_path: canonical,
                name: normalized,
                kind: EntryKind::File,
            });
        } else if metadata.is_dir() {
            if git {
                skipped_gitlinks += 1;
            } else {
                entries.push(Entry {
                    full_path,
                    name: format!("{normalized}/"),
                    kind: EntryKind::Directory,
                });
            }
        } else {
            skipped_missing += 1;
        }
    }
    Ok((entries, skipped_gitlinks, skipped_missing))
}

fn source_name(source: &Path) -> ToolResult<&str> {
    source
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| safe_relative(name))
        .ok_or_else(|| {
            ToolError::new(
                "justzip",
                format!("unsafe or non-UTF-8 input name: {}", source.display()),
            )
        })
}

fn git_folder(source: &Path) -> bool {
    source
        .ancestors()
        .any(|parent| parent.join(".git").exists())
}

fn git_binary() -> ToolResult<PathBuf> {
    if let Some(requested) = std::env::var_os("GIT_BIN").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(&requested);
        if path.is_file() {
            return Ok(path);
        }
        let text = requested.to_str().ok_or_else(|| {
            ToolError::new(
                "justzip",
                format!(
                    "GIT_BIN points to a missing non-UTF-8 path: {}",
                    path.to_string_lossy()
                ),
            )
        })?;
        deps::require("justzip", text)
    } else {
        deps::require("justzip", "git")
    }
}

fn folder_entries(source: &Path, destination: &Path) -> ToolResult<(Vec<Entry>, usize, usize)> {
    if git_folder(source) {
        let names = git_file_list(&git_binary()?, source)?;
        return collect_entries(source, destination, names, true);
    }
    let mut names = Vec::new();
    let mut nested = Vec::new();
    let mut skipped_gitlinks = 0;
    let mut skipped_missing = 0;
    let mut walker = walkdir::WalkDir::new(source)
        .min_depth(1)
        .sort_by_file_name()
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !entry.file_name().to_str().is_some_and(|name| {
                if cfg!(windows) {
                    name.eq_ignore_ascii_case(".git")
                } else {
                    name == ".git"
                }
            })
        });
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|error| ToolError::new("justzip", error.to_string()))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|error| ToolError::new("justzip", error.to_string()))?;
        let name = relative
            .to_str()
            .ok_or_else(|| ToolError::new("justzip", "cannot archive a non-UTF-8 path"))?;
        let name = if cfg!(windows) {
            name.replace('\\', "/")
        } else {
            name.to_owned()
        };
        names.push(name.clone());
        if entry.file_type().is_dir() && entry.path().join(".git").exists() {
            // An ordinary folder can contain repositories. Keep each one's
            // Git file set instead of descending into ignored build outputs.
            walker.skip_current_dir();
            let (mut entries, gitlinks, missing) = folder_entries(entry.path(), destination)?;
            for entry in &mut entries {
                entry.name = format!("{name}/{}", entry.name);
            }
            nested.extend(entries);
            skipped_gitlinks += gitlinks;
            skipped_missing += missing;
        }
    }
    let (mut entries, _, missing) = collect_entries(source, destination, names, false)?;
    entries.extend(nested);
    Ok((entries, skipped_gitlinks, skipped_missing + missing))
}

fn archive_entries(
    sources: &[PathBuf],
    destination: &Path,
) -> ToolResult<(Vec<Entry>, usize, usize)> {
    let mut entries = Vec::new();
    let mut skipped_gitlinks = 0;
    let mut skipped_missing = 0;
    for source in sources {
        if common::same_path(source, destination) {
            return Err(ToolError::new(
                "justzip",
                "the output would replace a selected source; choose a different --output",
            ));
        }
        let name = source_name(source)?;
        let metadata = fs::symlink_metadata(source)
            .map_err(|error| ToolError::new("justzip", error.to_string()))?;
        let (mut selected, gitlinks, missing) =
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let (mut selected, gitlinks, missing) = folder_entries(source, destination)?;
                if sources.len() > 1 {
                    for entry in &mut selected {
                        entry.name = format!("{name}/{}", entry.name);
                    }
                    selected.push(Entry {
                        full_path: source.clone(),
                        name: format!("{name}/"),
                        kind: EntryKind::Directory,
                    });
                }
                (selected, gitlinks, missing)
            } else {
                collect_entries(
                    source.parent().unwrap_or(Path::new(".")),
                    destination,
                    vec![name.to_owned()],
                    false,
                )?
            };
        entries.append(&mut selected);
        skipped_gitlinks += gitlinks;
        skipped_missing += missing;
    }
    // ZIP readers on Windows are case-insensitive. Reject both duplicate
    // names and a file that would shadow another entry's parent directory.
    let mut names = HashMap::new();
    for entry in &entries {
        let name = entry.name.trim_end_matches('/').to_lowercase();
        if names
            .insert(name, matches!(entry.kind, EntryKind::Directory))
            .is_some()
        {
            return Err(ToolError::new(
                "justzip",
                format!("conflicting archive name: {}", entry.name),
            ));
        }
    }
    for name in names.keys() {
        let mut parent = name.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if names.get(prefix) == Some(&false) {
                return Err(ToolError::new(
                    "justzip",
                    format!("conflicting archive name: {name}"),
                ));
            }
            parent = prefix;
        }
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok((entries, skipped_gitlinks, skipped_missing))
}

fn write_archive(
    entries: &[Entry],
    destination: &Path,
    compression: Compression,
) -> ToolResult<u64> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| ToolError::new("justzip", error.to_string()))?;
    let temporary = tempfile::Builder::new()
        .prefix(".justzip-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| ToolError::new("justzip", error.to_string()))?;
    let temporary_path = temporary.path().to_path_buf();
    let mut archive = zip::ZipWriter::new(temporary);
    for entry in entries {
        if matches!(entry.kind, EntryKind::Directory) {
            archive
                .add_directory(&entry.name, SimpleFileOptions::default())
                .map_err(|error| ToolError::new("justzip", error.to_string()))?;
            continue;
        }
        if let EntryKind::Symlink(target) = &entry.kind {
            let file_options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .unix_permissions(0o777);
            archive
                .add_symlink(
                    &entry.name,
                    target.to_string_lossy().replace('\\', "/"),
                    file_options,
                )
                .map_err(|error| ToolError::new("justzip", error.to_string()))?;
            continue;
        }
        let mut file = File::open(&entry.full_path).map_err(|error| {
            ToolError::new(
                "justzip",
                format!("cannot read {}: {error}", entry.full_path.display()),
            )
        })?;
        let metadata = file.metadata().map_err(|error| {
            ToolError::new(
                "justzip",
                format!("cannot inspect {}: {error}", entry.full_path.display()),
            )
        })?;
        let file_options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(compression.level()))
            .large_file(metadata.len() > u32::MAX as u64);
        #[cfg(unix)]
        let file_options = {
            use std::os::unix::fs::PermissionsExt;
            file_options.unix_permissions(metadata.permissions().mode())
        };
        archive
            .start_file(&entry.name, file_options)
            .map_err(|error| ToolError::new("justzip", error.to_string()))?;
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| ToolError::new("justzip", error.to_string()))?;
            if count == 0 {
                break;
            }
            archive
                .write_all(&buffer[..count])
                .map_err(|error| ToolError::new("justzip", error.to_string()))?;
        }
    }
    let temporary = archive
        .finish()
        .map_err(|error| ToolError::new("justzip", error.to_string()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| ToolError::new("justzip", error.to_string()))?;
    let bytes = temporary
        .as_file()
        .metadata()
        .map_err(|error| ToolError::new("justzip", error.to_string()))?
        .len();
    // Persist the temporary name before the shared replacement transaction.
    let (_file, persisted) = temporary.keep().map_err(|error| {
        ToolError::new(
            "justzip",
            format!("could not persist archive: {}", error.error),
        )
    })?;
    common::atomic_install("justzip", &persisted, destination)?;
    debug_assert_eq!(persisted, temporary_path);
    Ok(bytes)
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let options = parse(args)?;
    if options.help {
        println!("{HELP}");
        return Ok(());
    }
    let inputs = if options.sources.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        options.sources
    };
    let mut seen = HashSet::new();
    let sources: Vec<_> = inputs
        .iter()
        .map(|path| resolved_source(path))
        .collect::<ToolResult<Vec<_>>>()?
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect();
    let destination = destination(&sources, options.output.as_deref())?;
    let (entries, skipped_gitlinks, skipped_missing) = archive_entries(&sources, &destination)?;
    if options.dry_run {
        println!(
            "justzip: dry run — {} entries -> {} ({} compression)",
            entries.len(),
            common::display_path(&destination),
            options.compression.name()
        );
        for entry in entries.iter().take(100) {
            println!("  {}", entry.name);
        }
        if entries.len() > 100 {
            println!("  ... and {} more", entries.len() - 100);
        }
        if skipped_gitlinks > 0 {
            println!("justzip: skipped {skipped_gitlinks} gitlink(s)");
        }
        if skipped_missing > 0 {
            println!("justzip: skipped {skipped_missing} missing tracked path(s)");
        }
        return Ok(());
    }
    let started = Instant::now();
    let bytes = write_archive(&entries, &destination, options.compression)?;
    println!(
        "justzip: wrote {} — {} entries, {} in {:.1}s",
        common::display_path(&destination),
        entries.len(),
        common::format_bytes(bytes),
        started.elapsed().as_secs_f64()
    );
    if skipped_gitlinks > 0 {
        println!("justzip: skipped {skipped_gitlinks} gitlink(s)");
    }
    if skipped_missing > 0 {
        println!("justzip: skipped {skipped_missing} missing tracked path(s)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_git_paths() {
        assert!(!safe_relative("../secret"));
        assert!(!safe_relative("/rooted"));
        assert!(safe_relative("src/main.rs"));
    }

    #[test]
    fn parses_compression() {
        assert_eq!(Compression::parse("fast").unwrap().name(), "fast");
        assert_eq!(Compression::parse("bad").unwrap_err().exit_code(), 2);
    }

    #[test]
    fn symlinks_cannot_escape_the_archive_root() {
        assert!(safe_symlink_target(
            "assets/current",
            Path::new("icons/logo.svg")
        ));
        assert!(safe_symlink_target(
            "assets/current",
            Path::new("../logo.svg")
        ));
        assert!(!safe_symlink_target("current", Path::new("../secret")));
        assert!(!safe_symlink_target("current", Path::new("./../secret")));
        assert!(!safe_symlink_target("current", Path::new("/etc/passwd")));
        assert!(!safe_symlink_target("current", Path::new(r"..\secret")));
        assert!(!safe_symlink_target("current", Path::new(r"C:\secret")));
    }
}
