use std::collections::{BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::Utc;
use directories::BaseDirs;

use crate::commands::COMMANDS;
use crate::common;
use crate::error::{ToolError, ToolResult};

const MANIFEST_NAME: &str = ".justtools-install-manifest";
const MANIFEST_HEADER: &str = "JustTools native install manifest v1";
const INTERNAL_LEGACY_FILES: &[&str] = &[
    "just-audio.js",
    "just-avif.js",
    "just-common.js",
    "just-json.js",
    "just-media.js",
    "just-pdf.js",
    "just-qr.js",
    "just-svg.js",
];

struct Options {
    bin_directory: Option<PathBuf>,
    yes: bool,
    add_path: bool,
    help: bool,
}

#[derive(Debug)]
struct MovedFile {
    original: PathBuf,
    backup: PathBuf,
}

fn help() {
    println!(
        r#"just install — Install the compiled JustTools command aliases.

Usage:
  just install [options]

Stages the executable and every native alias before changing the destination.
An existing managed JustTools installation is moved to a timestamped backup;
failed upgrades are rolled back. Files not recognized as JustTools are never
replaced.

Options:
      --bin-dir DIR  Installation directory
  -y, --yes          Skip replacement and PATH confirmations
      --no-path      Do not offer to add the directory to PATH
  -h, --help         Show this help

Environment:
  JUSTTOOLS_BIN      Default installation directory override"#
    );
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        bin_directory: None,
        yes: false,
        add_path: true,
        help: false,
    };
    let mut index = 0;
    while index < args.len() {
        let argument = common::os_to_string("just", &args[index], "argument")?;
        match argument.as_str() {
            "-h" | "--help" => options.help = true,
            "-y" | "--yes" => options.yes = true,
            "--no-path" => options.add_path = false,
            "--bin-dir" => {
                index += 1;
                let value = args
                    .get(index)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| ToolError::usage("just", "--bin-dir needs a value"))?;
                options.bin_directory = Some(PathBuf::from(value));
            }
            _ if argument.starts_with("--bin-dir=") => {
                let value = argument.trim_start_matches("--bin-dir=");
                if value.is_empty() {
                    return Err(ToolError::usage("just", "--bin-dir needs a value"));
                }
                options.bin_directory = Some(PathBuf::from(value));
            }
            _ => {
                return Err(ToolError::usage(
                    "just",
                    format!("unknown install option: {argument}"),
                ));
            }
        }
        index += 1;
    }
    Ok(options)
}

fn default_bin_directory() -> ToolResult<PathBuf> {
    if let Some(value) = std::env::var_os("JUSTTOOLS_BIN") {
        return Ok(PathBuf::from(value));
    }
    #[cfg(windows)]
    {
        let existing = PathBuf::from(r"C:\cmd\bin");
        if existing.is_dir() {
            return Ok(existing);
        }
        let base = BaseDirs::new()
            .ok_or_else(|| ToolError::new("just", "cannot locate local data directory"))?;
        Ok(base.data_local_dir().join("JustTools/bin"))
    }
    #[cfg(not(windows))]
    {
        let base = BaseDirs::new()
            .ok_or_else(|| ToolError::new("just", "cannot locate home directory"))?;
        Ok(base.home_dir().join(".local/bin"))
    }
}

fn executable_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

fn command_names() -> Vec<&'static str> {
    let mut names: Vec<_> = COMMANDS.iter().map(|command| command.name).collect();
    names.extend(["just", "rmbg"]);
    names.sort_unstable();
    names.dedup();
    names
}

fn native_names() -> Vec<String> {
    command_names().into_iter().map(executable_name).collect()
}

fn legacy_candidate_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for command in command_names() {
        names.insert(command.to_owned());
        names.insert(format!("{command}.cmd"));
        names.insert(format!("{command}.ps1"));
        names.insert(format!("{command}.js"));
    }
    names.extend(INTERNAL_LEGACY_FILES.iter().map(|name| (*name).to_owned()));
    names
}

fn read_prefix(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(128 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

fn is_legacy_justtools(path: &Path) -> bool {
    let Some(bytes) = read_prefix(path) else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
    let internal_node_script = path
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| INTERNAL_LEGACY_FILES.contains(&name));
    match path.extension().and_then(OsStr::to_str) {
        Some(extension) if extension.eq_ignore_ascii_case("cmd") => {
            text.contains("@echo off") && text.contains("%~dp0") && text.contains("just")
        }
        Some(extension) if extension.eq_ignore_ascii_case("ps1") => {
            text.contains("just")
                && (text.contains("implementation")
                    || text.contains("usage:")
                    || text.contains("try 'just"))
        }
        Some(extension) if extension.eq_ignore_ascii_case("js") => {
            bytes.starts_with(b"#!/usr/bin/env node")
                && (internal_node_script || text.contains("just"))
        }
        None => bytes.starts_with(b"#!") && text.contains("just"),
        _ => false,
    }
}

fn legacy_files(bin: &Path) -> Vec<PathBuf> {
    legacy_candidate_names()
        .into_iter()
        .map(|name| bin.join(name))
        .filter(|path| path.is_file() && is_legacy_justtools(path))
        .collect()
}

fn files_identical(left: &Path, right: &Path) -> bool {
    let Ok(left_metadata) = fs::metadata(left) else {
        return false;
    };
    let Ok(right_metadata) = fs::metadata(right) else {
        return false;
    };
    if !left_metadata.is_file()
        || !right_metadata.is_file()
        || left_metadata.len() != right_metadata.len()
    {
        return false;
    }
    let (Ok(mut left), Ok(mut right)) = (File::open(left), File::open(right)) else {
        return false;
    };
    let mut left_buffer = [0_u8; 128 * 1024];
    let mut right_buffer = [0_u8; 128 * 1024];
    loop {
        let (Ok(left_count), Ok(right_count)) =
            (left.read(&mut left_buffer), right.read(&mut right_buffer))
        else {
            return false;
        };
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return false;
        }
        if left_count == 0 {
            return true;
        }
    }
}

fn manifest_contents() -> String {
    let mut contents = format!("{MANIFEST_HEADER}\n");
    for name in native_names() {
        contents.push_str(&name);
        contents.push('\n');
    }
    contents
}

fn discover_managed_files(source: &Path, bin: &Path) -> ToolResult<Vec<PathBuf>> {
    let allowed_names: HashSet<_> = native_names().into_iter().collect();
    let manifest = bin.join(MANIFEST_NAME);
    let mut managed = legacy_files(bin);

    if manifest.exists() {
        if !manifest.is_file() {
            return Err(ToolError::new(
                "just",
                format!(
                    "refusing to replace unrecognized path {}",
                    manifest.display()
                ),
            ));
        }
        let contents = fs::read_to_string(&manifest).map_err(|error| {
            ToolError::new(
                "just",
                format!("cannot read {}: {error}", manifest.display()),
            )
        })?;
        let mut lines = contents.lines();
        if lines.next() != Some(MANIFEST_HEADER) {
            return Err(ToolError::new(
                "just",
                format!(
                    "refusing to replace unrecognized manifest {}",
                    manifest.display()
                ),
            ));
        }
        for name in lines.filter(|name| allowed_names.contains(*name)) {
            let path = bin.join(name);
            if path.is_file() && !common::same_path(source, &path) {
                managed.push(path);
            }
        }
        managed.push(manifest.clone());
    } else {
        // Upgrade native installs made before manifests were introduced only
        // when at least one alias proves that the files are the same suite.
        let main = bin.join(executable_name("just"));
        if main.is_file() {
            let peers: Vec<_> = allowed_names
                .iter()
                .map(|name| bin.join(name))
                .filter(|path| !common::same_path(path, &main) && files_identical(&main, path))
                .collect();
            if !peers.is_empty() {
                if !common::same_path(source, &main) {
                    managed.push(main);
                }
                managed.extend(peers);
            }
        }
    }

    let managed_keys: HashSet<_> = managed.iter().map(|path| path_key(path)).collect();
    for name in &allowed_names {
        let path = bin.join(name);
        if path.exists()
            && !managed_keys.contains(&path_key(&path))
            && !common::same_path(source, &path)
        {
            return Err(ToolError::new(
                "just",
                format!(
                    "refusing to replace unrelated file {}; move it or choose another --bin-dir",
                    path.display()
                ),
            ));
        }
    }

    managed.sort_by(|left, right| left.as_os_str().cmp(right.as_os_str()));
    managed.dedup_by(|left, right| common::same_path(left, right));
    Ok(managed)
}

fn path_key(path: &Path) -> String {
    let text = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

fn unique_backup_directory(bin: &Path) -> PathBuf {
    let base = bin.join(".justtools-backups");
    let stamp = Utc::now().format("%Y%m%d-%H%M%S%.3f");
    for suffix in 0_u32.. {
        let name = if suffix == 0 {
            format!("{stamp}-{}", std::process::id())
        } else {
            format!("{stamp}-{}-{suffix}", std::process::id())
        };
        let candidate = base.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn restore_transaction(committed: &[PathBuf], moved: &[MovedFile]) -> Vec<String> {
    let mut failures = Vec::new();
    for path in committed.iter().rev() {
        if path.exists()
            && let Err(error) = fs::remove_file(path)
        {
            failures.push(format!("cannot remove new {}: {error}", path.display()));
        }
    }
    for file in moved.iter().rev() {
        if file.original.exists() {
            failures.push(format!(
                "cannot restore {} because a new path still exists; backup remains at {}",
                file.original.display(),
                file.backup.display()
            ));
            continue;
        }
        if let Err(error) = fs::rename(&file.backup, &file.original) {
            failures.push(format!(
                "cannot restore {} from {}: {error}",
                file.original.display(),
                file.backup.display()
            ));
        }
    }
    failures
}

fn transaction_error(
    action: &str,
    error: impl std::fmt::Display,
    backup: Option<&Path>,
    rollback_failures: Vec<String>,
) -> ToolError {
    if rollback_failures.is_empty() {
        ToolError::new(
            "just",
            format!("{action}: {error}; previous installation restored"),
        )
    } else {
        let recovery = backup.map_or_else(
            || "no backup directory was created".to_owned(),
            |path| format!("recovery files remain in {}", path.display()),
        );
        ToolError::new(
            "just",
            format!(
                "{action}: {error}; rollback incomplete ({recovery}): {}",
                rollback_failures.join("; ")
            ),
        )
    }
}

fn stage_installation(source: &Path, bin: &Path) -> ToolResult<tempfile::TempDir> {
    if !source.is_file() {
        return Err(ToolError::new(
            "just",
            format!("installer source is not a file: {}", source.display()),
        ));
    }
    let stage = tempfile::Builder::new()
        .prefix(".justtools-stage-")
        .tempdir_in(bin)
        .map_err(|error| ToolError::new("just", format!("cannot stage installation: {error}")))?;
    let main_name = executable_name("just");
    let staged_main = stage.path().join(&main_name);
    fs::copy(source, &staged_main).map_err(|error| {
        ToolError::new(
            "just",
            format!("cannot stage {}: {error}", source.display()),
        )
    })?;
    for name in native_names() {
        if name == main_name {
            continue;
        }
        let alias = stage.path().join(name);
        if fs::hard_link(&staged_main, &alias).is_err() {
            fs::copy(&staged_main, &alias).map_err(|error| {
                ToolError::new(
                    "just",
                    format!("cannot stage alias {}: {error}", alias.display()),
                )
            })?;
        }
    }
    fs::write(stage.path().join(MANIFEST_NAME), manifest_contents())
        .map_err(|error| ToolError::new("just", format!("cannot stage manifest: {error}")))?;
    Ok(stage)
}

fn install_transaction(
    source: &Path,
    bin: &Path,
    managed: &[PathBuf],
    fail_after_for_test: Option<usize>,
) -> ToolResult<Option<PathBuf>> {
    let stage = stage_installation(source, bin)?;
    let backup = (!managed.is_empty()).then(|| unique_backup_directory(bin));
    let mut moved = Vec::new();
    if let Some(backup) = &backup {
        fs::create_dir_all(backup).map_err(|error| {
            ToolError::new(
                "just",
                format!(
                    "cannot create recovery directory {}: {error}",
                    backup.display()
                ),
            )
        })?;
        for original in managed {
            let Some(name) = original.file_name() else {
                let failures = restore_transaction(&[], &moved);
                return Err(transaction_error(
                    "cannot back up managed path",
                    original.display(),
                    Some(backup),
                    failures,
                ));
            };
            let destination = backup.join(name);
            if let Err(error) = fs::rename(original, &destination) {
                let failures = restore_transaction(&[], &moved);
                return Err(transaction_error(
                    &format!("cannot back up {}", original.display()),
                    error,
                    Some(backup),
                    failures,
                ));
            }
            moved.push(MovedFile {
                original: original.clone(),
                backup: destination,
            });
        }
    }

    let main_name = executable_name("just");
    let mut targets = native_names();
    targets.retain(|name| name != &main_name);
    targets.insert(0, main_name.clone());
    targets.push(MANIFEST_NAME.to_owned());
    let mut committed = Vec::new();
    for name in targets {
        let destination = bin.join(&name);
        if name == main_name && common::same_path(source, &destination) {
            continue;
        }
        if fail_after_for_test.is_some_and(|limit| committed.len() >= limit) {
            let failures = restore_transaction(&committed, &moved);
            return Err(transaction_error(
                "cannot commit staged installation",
                "injected test failure",
                backup.as_deref(),
                failures,
            ));
        }
        let staged = stage.path().join(&name);
        if let Err(error) = fs::rename(&staged, &destination) {
            let failures = restore_transaction(&committed, &moved);
            return Err(transaction_error(
                &format!("cannot install {}", destination.display()),
                error,
                backup.as_deref(),
                failures,
            ));
        }
        committed.push(destination);
    }
    Ok(backup)
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let options = parse(args)?;
    if options.help {
        help();
        return Ok(());
    }
    let source = std::env::current_exe()
        .map_err(|error| ToolError::new("just", format!("cannot locate executable: {error}")))?;
    let bin = options.bin_directory.unwrap_or(default_bin_directory()?);
    fs::create_dir_all(&bin).map_err(|error| {
        ToolError::new("just", format!("cannot create {}: {error}", bin.display()))
    })?;
    let managed = discover_managed_files(&source, &bin)?;
    if !managed.is_empty()
        && !options.yes
        && !common::confirm(
            "just",
            &format!(
                "just: replace the managed JustTools installation in {}",
                bin.display()
            ),
        )?
    {
        return Err(ToolError::cancelled("just"));
    }
    let backup = install_transaction(&source, &bin, &managed, None)?;
    println!(
        "just: installed {} compiled command aliases in {}",
        command_names().len(),
        bin.display()
    );
    if let Some(backup) = backup {
        println!(
            "just: previous managed installation backed up to {}",
            backup.display()
        );
    }
    if options.add_path && !crate::pathing::contains(&bin) {
        let add = options.yes
            || common::confirm("just", &format!("just: add {} to PATH", bin.display()))?;
        if add {
            crate::pathing::add(&bin)?;
        } else {
            println!("just: PATH unchanged; add {} manually", bin.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(bin: &Path) {
        fs::write(bin.join(MANIFEST_NAME), manifest_contents()).unwrap();
    }

    #[test]
    fn aliases_cover_every_command() {
        let names = command_names();
        assert!(names.contains(&"just"));
        assert!(names.contains(&"rmbg"));
        assert!(names.contains(&"justrmbg"));
        assert!(names.contains(&"justvideo"));
    }

    #[test]
    fn recognizes_legacy_scripts_but_not_arbitrary_collisions() {
        let directory = tempfile::tempdir().unwrap();
        let internal = directory.path().join("just-common.js");
        let unrelated = directory.path().join("justvideo.cmd");
        fs::write(&internal, b"#!/usr/bin/env node\n'use strict';\n").unwrap();
        fs::write(&unrelated, b"@echo off\necho unrelated\n").unwrap();
        assert!(is_legacy_justtools(&internal));
        assert!(!is_legacy_justtools(&unrelated));
    }

    #[test]
    fn unrelated_collisions_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("downloaded");
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(&source, b"new").unwrap();
        fs::write(bin.join(executable_name("just")), b"unrelated").unwrap();
        let error = discover_managed_files(&source, &bin).unwrap_err();
        assert!(error.message().contains("unrelated file"));
        assert_eq!(
            fs::read(bin.join(executable_name("just"))).unwrap(),
            b"unrelated"
        );
    }

    #[test]
    fn failed_commit_restores_every_managed_file() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("downloaded");
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(&source, b"new suite").unwrap();
        let main = bin.join(executable_name("just"));
        let alias = bin.join(executable_name("justaudio"));
        fs::write(&main, b"old suite").unwrap();
        fs::write(&alias, b"old suite").unwrap();
        write_manifest(&bin);

        let managed = discover_managed_files(&source, &bin).unwrap();
        let error = install_transaction(&source, &bin, &managed, Some(1)).unwrap_err();
        assert!(error.message().contains("previous installation restored"));
        assert_eq!(fs::read(main).unwrap(), b"old suite");
        assert_eq!(fs::read(alias).unwrap(), b"old suite");
        assert!(bin.join(MANIFEST_NAME).is_file());
    }

    #[test]
    fn successful_install_creates_manifest_and_every_alias() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("downloaded");
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(&source, b"native suite").unwrap();
        let managed = discover_managed_files(&source, &bin).unwrap();
        assert!(
            install_transaction(&source, &bin, &managed, None)
                .unwrap()
                .is_none()
        );
        for name in native_names() {
            assert_eq!(fs::read(bin.join(name)).unwrap(), b"native suite");
        }
        assert!(bin.join(MANIFEST_NAME).is_file());
    }

    #[test]
    fn reinstall_from_destination_never_moves_running_main() {
        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let main = bin.join(executable_name("just"));
        fs::write(&main, b"running suite").unwrap();
        let managed = discover_managed_files(&main, &bin).unwrap();
        install_transaction(&main, &bin, &managed, None).unwrap();
        assert_eq!(fs::read(&main).unwrap(), b"running suite");
        assert_eq!(
            fs::read(bin.join(executable_name("justvideo"))).unwrap(),
            b"running suite"
        );
    }
}
