//! Source-preserving filtered copies, fully planned before any output is created.
use super::Settings;
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Plan {
    pub entries: Vec<(PathBuf, PathBuf, bool)>,
    pub skipped: usize,
}

fn excluded(relative: &Path, patterns: &[String]) -> bool {
    let value = relative.to_string_lossy().replace('\\', "/");
    patterns.iter().any(|pattern| {
        let pattern = pattern.replace('\\', "/");
        if pattern.contains('/') {
            value.eq_ignore_ascii_case(&pattern)
                || value
                    .to_ascii_lowercase()
                    .starts_with(&(pattern.to_ascii_lowercase() + "/"))
        } else {
            relative.components().any(|part| {
                part.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&pattern)
            })
        }
    })
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn plan(sources: &[PathBuf], output: &Path, settings: &Settings) -> Result<Plan, String> {
    let output = output
        .canonicalize()
        .map_err(|e| format!("cannot resolve destination: {e}"))?;
    let mut plan = Plan {
        entries: Vec::new(),
        skipped: 0,
    };
    let mut destinations = HashSet::new();
    for source in sources {
        let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if is_link(&metadata) {
            return Err(format!(
                "selected links/junctions are not copied: {}",
                source.display()
            ));
        }
        let source = source.canonicalize().map_err(|e| e.to_string())?;
        if metadata.is_dir() && output.starts_with(&source) {
            return Err("cannot copy a folder into itself or one of its descendants".into());
        }
        let name = source.file_name().ok_or("cannot copy a filesystem root")?;
        let relative_root = Path::new(name);
        if excluded(relative_root, &settings.excludes) {
            plan.skipped += 1;
            continue;
        }
        if settings.gitignore && ignored_file(&source)? {
            plan.skipped += 1;
            continue;
        }
        let destination = super::free_path(&output, &name.to_string_lossy());
        if !destinations.insert(destination.to_string_lossy().to_lowercase()) {
            return Err(format!(
                "selected sources collide at {}",
                destination.display()
            ));
        }
        if metadata.is_file() {
            plan.entries.push((source, destination, false));
            continue;
        }
        let mut walk = ignore::WalkBuilder::new(&source);
        walk.hidden(false)
            .follow_links(false)
            .git_ignore(settings.gitignore)
            .git_global(false)
            .git_exclude(settings.gitignore)
            .ignore(false)
            .parents(settings.gitignore)
            .require_git(false);
        let root = source.clone();
        let patterns = settings.excludes.clone();
        walk.filter_entry(move |entry| {
            !fs::symlink_metadata(entry.path()).is_ok_and(|metadata| is_link(&metadata))
                && (entry.depth() == 0
                    || !excluded(
                        entry.path().strip_prefix(&root).unwrap_or(entry.path()),
                        &patterns,
                    ))
        });
        for entry in walk.build() {
            if crate::common::interrupted() {
                return Err("cancelled".into());
            }
            let entry = entry.map_err(|e| format!("cannot plan copy: {e}"))?;
            if let Some(error) = entry.error() {
                return Err(format!("invalid ignore rules: {error}"));
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if is_link(&metadata) {
                plan.skipped += 1;
                continue;
            }
            if !metadata.is_dir() && !metadata.is_file() {
                return Err("unsupported special file in copy".into());
            }
            let relative = entry
                .path()
                .strip_prefix(&source)
                .map_err(|e| e.to_string())?;
            plan.entries.push((
                entry.path().to_owned(),
                destination.join(relative),
                metadata.is_dir(),
            ));
        }
    }
    // Detect overlapping selections before creating anything.
    let mut all = HashSet::new();
    for (_, target, _) in &plan.entries {
        if !all.insert(target.to_string_lossy().to_lowercase()) {
            return Err("overlapping copy destinations".into());
        }
        if target.exists() {
            return Err(format!(
                "destination appeared while planning: {}",
                target.display()
            ));
        }
    }
    Ok(plan)
}

fn ignored_file(source: &Path) -> Result<bool, String> {
    let Some(parent) = source.parent() else {
        return Ok(false);
    };
    // A walker handles ancestor-relative matching correctly, including negation.
    let mut walk = ignore::WalkBuilder::new(parent);
    walk.max_depth(Some(1))
        .hidden(false)
        .ignore(false)
        .git_global(false)
        .require_git(false);
    for entry in walk.build() {
        let entry = entry.map_err(|e| e.to_string())?;
        if let Some(error) = entry.error() {
            return Err(format!("invalid ignore rules: {error}"));
        }
        if entry.path() == source {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn execute(plan: &Plan) -> Result<usize, String> {
    let mut count = 0;
    for (source, destination, directory) in &plan.entries {
        crate::progress::report(
            "Copying files…",
            source.file_name().unwrap_or_default().to_string_lossy(),
            None,
        );
        if crate::common::interrupted() {
            return Err(format!("cancelled; {count} completed files kept"));
        }
        // Recheck before opening: never follow a source swapped for a junction/symlink.
        if is_link(&fs::symlink_metadata(source).map_err(|e| e.to_string())?) {
            return Err(format!("source changed to a link: {}", source.display()));
        }
        if *directory {
            fs::create_dir(destination)
                .map_err(|e| format!("cannot create {}: {e}", destination.display()))?;
            continue;
        }
        let parent = destination.parent().ok_or("missing destination parent")?;
        let mut temp = tempfile::Builder::new()
            .prefix(".justpaste-")
            .tempfile_in(parent)
            .map_err(|e| e.to_string())?;
        let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
        let mut buffer = vec![0; 256 * 1024];
        loop {
            if crate::common::interrupted() {
                return Err(format!("cancelled; {count} completed files kept"));
            }
            let size = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if size == 0 {
                break;
            }
            temp.write_all(&buffer[..size]).map_err(|e| e.to_string())?;
        }
        temp.flush().map_err(|e| e.to_string())?;
        temp.persist_noclobber(destination)
            .map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copies_with_nested_negation_exclusions_empty_folders_and_no_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        let output = temp.path().join("out");
        for dir in ["node_modules/a", "nested", "empty"] {
            fs::create_dir_all(source.join(dir)).unwrap();
        }
        fs::create_dir(&output).unwrap();
        fs::write(source.join(".gitignore"), "*.log\n!keep.log\n").unwrap();
        fs::write(source.join("nested/.gitignore"), "!nested.log\n").unwrap();
        for file in [
            "a.txt",
            "drop.log",
            "keep.log",
            "nested/nested.log",
            "node_modules/a/x.js",
        ] {
            fs::write(source.join(file), file).unwrap();
        }
        let settings = Settings {
            gitignore: true,
            excludes: vec!["node_modules".into()],
            ..Default::default()
        };
        let plan = plan(std::slice::from_ref(&source), &output, &settings).unwrap();
        execute(&plan).unwrap();
        assert!(output.join("project/empty").is_dir());
        assert!(output.join("project/keep.log").is_file());
        assert!(output.join("project/nested/nested.log").is_file());
        assert!(!output.join("project/drop.log").exists());
        assert!(!output.join("project/node_modules").exists());
        assert!(source.join("node_modules/a/x.js").exists());
        assert!(execute(&plan).is_err());
    }
    #[test]
    fn refuses_self_copy_and_colliding_selected_names() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        let out = temp.path().join("out");
        for p in [&a, &b, &out] {
            fs::create_dir(p).unwrap();
        }
        fs::write(a.join("same"), "a").unwrap();
        fs::write(b.join("same"), "b").unwrap();
        assert!(plan(std::slice::from_ref(&a), &a, &Settings::default()).is_err());
        assert!(
            plan(
                &[a.join("same"), b.join("same")],
                &out,
                &Settings::default()
            )
            .is_err()
        );
    }
}
