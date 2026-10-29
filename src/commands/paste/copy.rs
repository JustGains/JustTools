//! Source-preserving filtered copies, fully planned before any output is created.
use super::Settings;
use rayon::prelude::*;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
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

/// Creates the planned folders, then copies the files with the operating
/// system's own copy (CopyFileEx on Windows: block cloning, server-side SMB
/// copies, timestamps and attributes), several at a time so a tree of small
/// files is not paced by one file's latency.
pub fn execute(plan: &Plan) -> Result<usize, String> {
    // The walk lists every folder before its contents.
    for (source, destination, _) in plan.entries.iter().filter(|entry| entry.2) {
        if crate::common::interrupted() {
            return Err("cancelled; 0 completed files kept".into());
        }
        unchanged(source)?;
        fs::create_dir(destination)
            .map_err(|e| format!("cannot create {}: {e}", destination.display()))?;
    }
    let files: Vec<_> = plan.entries.iter().filter(|entry| !entry.2).collect();
    let total_bytes: u64 = files
        .iter()
        .filter_map(|(source, ..)| fs::metadata(source).ok())
        .map(|metadata| metadata.len())
        .sum();
    let copied = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let report = |name: &std::ffi::OsStr| {
        let percent = (total_bytes > 0)
            .then(|| (bytes.load(Ordering::Relaxed).min(total_bytes) * 100 / total_bytes) as u32);
        crate::progress::report(
            "Copying files…",
            format!(
                "{} of {} · {}",
                copied.load(Ordering::Relaxed),
                files.len(),
                name.to_string_lossy()
            ),
            percent,
        );
    };
    let reporter = crate::progress::current();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| e.to_string())?;
    let result = pool.install(|| {
        files.par_iter().try_for_each(|(source, destination, _)| {
            crate::progress::within(reporter.as_ref(), || {
                if crate::common::interrupted() {
                    return Err(CANCELLED.to_owned());
                }
                let name = source.file_name().unwrap_or_default();
                report(name);
                let mut last = 0;
                copy_file(source, destination, &mut |done| {
                    bytes.fetch_add(done.saturating_sub(last), Ordering::Relaxed);
                    last = done;
                    report(name);
                    !crate::common::interrupted()
                })?;
                copied.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
        })
    });
    let count = copied.into_inner();
    match result {
        Ok(()) => Ok(count),
        Err(error) if error == CANCELLED || crate::common::interrupted() => {
            Err(format!("cancelled; {count} completed files kept"))
        }
        Err(error) => Err(error),
    }
}

const CANCELLED: &str = "cancelled";

/// Never follow a source swapped for a junction/symlink since planning.
fn unchanged(source: &Path) -> Result<(), String> {
    if is_link(&fs::symlink_metadata(source).map_err(|e| e.to_string())?) {
        return Err(format!("source changed to a link: {}", source.display()));
    }
    Ok(())
}

/// Copies under a hidden `.justpaste-*` name beside the destination and
/// publishes it only when complete, never over an existing file. `progress`
/// gets the bytes copied so far and returns false to cancel.
fn copy_file(
    source: &Path,
    destination: &Path,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(), String> {
    unchanged(source)?;
    let parent = destination.parent().ok_or("missing destination parent")?;
    let temp = tempfile::Builder::new()
        .prefix(".justpaste-")
        .make_in(parent, |path| system_copy(source, path, &mut *progress))
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::Interrupted {
                CANCELLED.to_owned()
            } else {
                format!("cannot copy {}: {e}", source.display())
            }
        })?;
    temp.persist_noclobber(destination)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// CopyFileEx into a path that must not exist yet; an existing one reports
/// AlreadyExists, so the temporary name is retried.
#[cfg(windows)]
fn system_copy(
    source: &Path,
    target: &Path,
    progress: &mut dyn FnMut(u64) -> bool,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{Foundation::HANDLE, Storage::FileSystem::*};
    unsafe extern "system" fn routine(
        _total: i64,
        transferred: i64,
        _stream_size: i64,
        _stream_transferred: i64,
        _stream: u32,
        _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
        _source: HANDLE,
        _target: HANDLE,
        data: *const std::ffi::c_void,
    ) -> COPYPROGRESSROUTINE_PROGRESS {
        let progress = unsafe { &mut *(data as *mut &mut dyn FnMut(u64) -> bool) };
        if progress(transferred.max(0) as u64) {
            PROGRESS_CONTINUE
        } else {
            PROGRESS_CANCEL
        }
    }
    let wide =
        |path: &Path| -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (from, to) = (wide(source), wide(target));
    let mut progress = progress;
    let copied = unsafe {
        CopyFileExW(
            from.as_ptr(),
            to.as_ptr(),
            Some(routine),
            &mut progress as *mut &mut dyn FnMut(u64) -> bool as *const std::ffi::c_void,
            std::ptr::null_mut(),
            COPY_FILE_FAIL_IF_EXISTS,
        )
    };
    if copied != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return Err(error);
    }
    let _ = fs::remove_file(target);
    // ERROR_REQUEST_ABORTED: the progress routine cancelled.
    if error.raw_os_error() == Some(1235) {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    Err(error)
}

/// The platform copy (cloning or copy_file_range where available), keeping
/// the modified time. Cancellation applies between files.
#[cfg(not(windows))]
fn system_copy(
    source: &Path,
    target: &Path,
    progress: &mut dyn FnMut(u64) -> bool,
) -> std::io::Result<()> {
    drop(fs::File::create_new(target)?);
    let result = (|| {
        let copied = fs::copy(source, target)?;
        if let Ok(modified) = fs::metadata(source).and_then(|m| m.modified()) {
            fs::File::options()
                .write(true)
                .open(target)?
                .set_modified(modified)?;
        }
        if !progress(copied) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(target);
    }
    result
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
    fn copies_many_files_keeping_contents_and_modified_times() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("tree");
        let output = temp.path().join("out");
        fs::create_dir_all(source.join("a/b")).unwrap();
        fs::create_dir(&output).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        for index in 0..200 {
            let file = source
                .join(if index % 2 == 0 { "a" } else { "a/b" })
                .join(format!("{index}.txt"));
            fs::write(&file, index.to_string()).unwrap();
            fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        let plan = plan(std::slice::from_ref(&source), &output, &Settings::default()).unwrap();
        assert_eq!(execute(&plan).unwrap(), 200);
        for index in 0..200 {
            let file = output
                .join("tree")
                .join(if index % 2 == 0 { "a" } else { "a/b" })
                .join(format!("{index}.txt"));
            assert_eq!(fs::read_to_string(&file).unwrap(), index.to_string());
            assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), old);
        }
        let leftovers = fs::read_dir(output.join("tree/a")).unwrap().flatten();
        assert!(
            !leftovers
                .into_iter()
                .any(|e| e.file_name().to_string_lossy().starts_with(".justpaste-"))
        );
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
