use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
};

use sysinfo::{
    Pid, ProcessRefreshKind, ProcessesToUpdate, SUPPORTED_SIGNALS, Signal, System, UpdateKind,
    get_current_pid,
};

use super::model::{KillTarget, ProcessInfo, Runtime, WorkloadIdentity};

#[derive(Debug)]
pub struct ScanResult {
    pub processes: Vec<ProcessInfo>,
    pub launcher_ancestry: HashSet<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationOutcome {
    GracefulRequested,
    ForceRequested,
    Failed,
    Changed,
}

#[derive(Clone, Debug)]
struct ProjectMetadata {
    root: PathBuf,
    name: String,
}

#[derive(Clone, Debug)]
struct RawProcess {
    pid: u32,
    parent_pid: Option<u32>,
    runtime: Runtime,
    process_name: String,
    executable: Option<PathBuf>,
    cwd: Option<PathBuf>,
    args: Vec<String>,
    cpu_percent: f32,
    memory_bytes: u64,
    virtual_memory_bytes: u64,
    disk_read_bytes: u64,
    disk_written_bytes: u64,
    start_time: u64,
    run_time: u64,
    status: String,
}

pub struct ProcessScanner {
    system: System,
    project_cache: HashMap<PathBuf, Option<ProjectMetadata>>,
}

impl ProcessScanner {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            project_cache: HashMap::new(),
        }
    }

    pub fn scan(&mut self) -> ScanResult {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );

        let launcher_ancestry = current_ancestry(&self.system);
        let raw_processes = self
            .system
            .processes()
            .values()
            .filter_map(raw_process)
            .collect::<Vec<_>>();
        let processes = raw_processes
            .into_iter()
            .map(|raw| self.enrich(raw))
            .collect();

        ScanResult {
            processes,
            launcher_ancestry,
        }
    }

    pub fn terminate(&self, target: &KillTarget, force: bool) -> TerminationOutcome {
        let Some(process) = self.system.process(Pid::from_u32(target.pid)) else {
            return TerminationOutcome::Changed;
        };
        if process.start_time() != target.start_time
            || classify_process(process) != Some(target.runtime)
        {
            return TerminationOutcome::Changed;
        }

        if force {
            return if process.kill() {
                TerminationOutcome::ForceRequested
            } else {
                TerminationOutcome::Failed
            };
        }

        if SUPPORTED_SIGNALS.contains(&Signal::Term) {
            return match process.kill_with(Signal::Term) {
                Some(true) => TerminationOutcome::GracefulRequested,
                Some(false) | None => TerminationOutcome::Failed,
            };
        }

        if process.kill() {
            TerminationOutcome::ForceRequested
        } else {
            TerminationOutcome::Failed
        }
    }

    fn enrich(&mut self, raw: RawProcess) -> ProcessInfo {
        let project = raw.cwd.as_deref().and_then(|cwd| self.project_for(cwd));
        let anchor_path = project
            .as_ref()
            .map(|project| project.root.as_path())
            .or(raw.cwd.as_deref());
        let project_root = anchor_path.map(normalize_path);
        let project_name = project
            .as_ref()
            .map(|project| project.name.clone())
            .or_else(|| {
                anchor_path
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| raw.process_name.clone());
        let executable = raw.executable.as_deref().map(normalize_path);
        let cwd = raw.cwd.as_deref().map(normalize_path);
        let (workload, workload_label, uses_project_anchor) =
            derive_workload(raw.runtime, &raw.args, raw.cwd.as_deref(), anchor_path);
        let command = format_command(&raw.args);
        let identity = WorkloadIdentity {
            runtime: raw.runtime,
            executable: executable.clone(),
            anchor: uses_project_anchor.then(|| project_root.clone()).flatten(),
            workload,
        };

        ProcessInfo {
            pid: raw.pid,
            parent_pid: raw.parent_pid,
            runtime: raw.runtime,
            process_name: raw.process_name,
            executable,
            cwd,
            command,
            args: raw.args,
            cpu_percent: raw.cpu_percent,
            memory_bytes: raw.memory_bytes,
            virtual_memory_bytes: raw.virtual_memory_bytes,
            disk_read_bytes: raw.disk_read_bytes,
            disk_written_bytes: raw.disk_written_bytes,
            start_time: raw.start_time,
            run_time: raw.run_time,
            status: raw.status,
            project_name,
            project_root,
            workload_label,
            identity,
        }
    }

    fn project_for(&mut self, cwd: &Path) -> Option<ProjectMetadata> {
        if let Some(cached) = self.project_cache.get(cwd) {
            return cached.clone();
        }
        let discovered = discover_project(cwd);
        self.project_cache
            .insert(cwd.to_path_buf(), discovered.clone());
        discovered
    }
}

fn raw_process(process: &sysinfo::Process) -> Option<RawProcess> {
    let runtime = classify_process(process)?;
    let disk = process.disk_usage();
    Some(RawProcess {
        pid: process.pid().as_u32(),
        parent_pid: process.parent().map(Pid::as_u32),
        runtime,
        process_name: process.name().to_string_lossy().into_owned(),
        executable: process.exe().map(Path::to_path_buf),
        cwd: process.cwd().map(Path::to_path_buf),
        args: process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
        cpu_percent: process.cpu_usage(),
        memory_bytes: process.memory(),
        virtual_memory_bytes: process.virtual_memory(),
        disk_read_bytes: disk.read_bytes,
        disk_written_bytes: disk.written_bytes,
        start_time: process.start_time(),
        run_time: process.run_time(),
        status: format!("{:?}", process.status()).to_lowercase(),
    })
}

fn classify_process(process: &sysinfo::Process) -> Option<Runtime> {
    if let Some(executable) = process.exe().filter(|path| !path.as_os_str().is_empty()) {
        if let Some(runtime) = classify_runtime_token(executable.as_os_str()) {
            return Some(runtime);
        }
        // A trustworthy executable path says this is not a runtime, even if argv[0]
        // happens to look like one.
        return None;
    }

    classify_runtime_token(process.name()).or_else(|| {
        process
            .cmd()
            .first()
            .and_then(|value| classify_runtime_token(value))
    })
}

fn classify_runtime_token(token: &OsStr) -> Option<Runtime> {
    let token = token.to_string_lossy();
    let file_name = Path::new(token.trim_matches('"'))
        .file_name()
        .unwrap_or_else(|| OsStr::new(token.as_ref()))
        .to_string_lossy()
        .to_lowercase();
    let stem = file_name.strip_suffix(".exe").unwrap_or(&file_name);

    if matches!(stem, "node" | "nodejs") {
        return Some(Runtime::Node);
    }
    if matches!(stem, "bun" | "bunx" | "bun-debug") {
        return Some(Runtime::Bun);
    }
    if matches!(stem, "py" | "pypy" | "pypy3" | "python" | "pythonw")
        || versioned_runtime(stem, "python")
        || versioned_runtime(stem, "pythonw")
        || versioned_runtime(stem, "pypy")
    {
        return Some(Runtime::Python);
    }
    None
}

fn versioned_runtime(value: &str, prefix: &str) -> bool {
    let Some(version) = value.strip_prefix(prefix) else {
        return false;
    };
    !version.is_empty()
        && version.chars().any(|character| character.is_ascii_digit())
        && version
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}

fn current_ancestry(system: &System) -> HashSet<u32> {
    let mut ancestry = HashSet::new();
    let Ok(mut current) = get_current_pid() else {
        return ancestry;
    };

    while ancestry.insert(current.as_u32()) {
        let Some(parent) = system.process(current).and_then(sysinfo::Process::parent) else {
            break;
        };
        current = parent;
    }
    ancestry
}

fn discover_project(cwd: &Path) -> Option<ProjectMetadata> {
    let mut current = Some(cwd);
    for _ in 0..16 {
        let directory = current?;
        if is_project_root(directory) {
            return Some(ProjectMetadata {
                root: directory.to_path_buf(),
                name: read_project_name(directory).unwrap_or_else(|| {
                    directory
                        .file_name()
                        .unwrap_or(directory.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                }),
            });
        }
        current = directory.parent();
    }
    None
}

fn is_project_root(directory: &Path) -> bool {
    [
        "package.json",
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "requirements.txt",
        "Pipfile",
        "uv.lock",
        "bun.lock",
        "bun.lockb",
        ".git",
    ]
    .iter()
    .any(|marker| directory.join(marker).exists())
}

fn read_project_name(directory: &Path) -> Option<String> {
    let package_json = directory.join("package.json");
    if let Some(source) = read_small_file(&package_json)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&source)
        && let Some(name) = value.get("name").and_then(serde_json::Value::as_str)
    {
        return Some(name.to_owned());
    }

    let pyproject = directory.join("pyproject.toml");
    if let Some(source) = read_small_file(&pyproject)
        && let Ok(value) = toml::from_str::<toml::Value>(&source)
    {
        if let Some(name) = value
            .get("project")
            .and_then(|project| project.get("name"))
            .and_then(toml::Value::as_str)
        {
            return Some(name.to_owned());
        }
        if let Some(name) = value
            .get("tool")
            .and_then(|tool| tool.get("poetry"))
            .and_then(|poetry| poetry.get("name"))
            .and_then(toml::Value::as_str)
        {
            return Some(name.to_owned());
        }
    }
    None
}

fn read_small_file(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    (metadata.len() <= 1_048_576)
        .then(|| fs::read_to_string(path).ok())
        .flatten()
}

fn derive_workload(
    runtime: Runtime,
    command: &[String],
    cwd: Option<&Path>,
    anchor: Option<&Path>,
) -> (String, String, bool) {
    let args = runtime_arguments(runtime, command);
    match runtime {
        Runtime::Node => derive_node_workload(args, cwd, anchor),
        Runtime::Bun
            if command
                .first()
                .is_some_and(|value| file_stem_lower(value) == "bunx") =>
        {
            derive_bunx_workload(args, anchor)
        }
        Runtime::Bun => derive_bun_workload(args, cwd, anchor),
        Runtime::Python => derive_python_workload(args, cwd, anchor),
    }
}

fn runtime_arguments(runtime: Runtime, command: &[String]) -> &[String] {
    if command
        .first()
        .and_then(|arg| classify_runtime_token(OsStr::new(arg)))
        == Some(runtime)
    {
        &command[1..]
    } else {
        command
    }
}

fn derive_node_workload(
    args: &[String],
    cwd: Option<&Path>,
    anchor: Option<&Path>,
) -> (String, String, bool) {
    let Some(index) = first_node_entrypoint(args) else {
        return (
            "interactive".into(),
            "node interactive".into(),
            anchor.is_some(),
        );
    };
    let entrypoint = &args[index];
    let base = file_stem_lower(entrypoint);

    if matches!(
        base.as_str(),
        "npm-cli" | "npx-cli" | "pnpm" | "pnpx" | "yarn" | "yarnpkg"
    ) {
        let tool = base.trim_end_matches("-cli");
        let tail = &args[index + 1..];
        let subcommand = tail
            .iter()
            .position(|arg| !arg.starts_with('-'))
            .map(|position| position + index + 1);
        if let Some(subcommand_index) = subcommand {
            let subcommand = &args[subcommand_index];
            let script = args
                .get(subcommand_index + 1)
                .filter(|arg| !arg.starts_with('-'));
            let suffix = script
                .map(|script| format!(":{script}"))
                .unwrap_or_default();
            let label_suffix = script
                .map(|script| format!(" {script}"))
                .unwrap_or_default();
            return (
                format!("tool:{tool}:{subcommand}{suffix}"),
                format!("{tool} {subcommand}{label_suffix}"),
                anchor.is_some(),
            );
        }
    }

    let (normalized, anchored) = normalize_script(entrypoint, cwd, anchor);
    let friendly = friendly_script_name(&normalized);
    (
        format!("script:{normalized}"),
        format!("node {friendly}"),
        anchored,
    )
}

fn first_node_entrypoint(args: &[String]) -> Option<usize> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            return (index + 1 < args.len()).then_some(index + 1);
        }
        if !arg.starts_with('-') || arg == "-" {
            return Some(index);
        }
        let takes_value = matches!(
            arg.as_str(),
            "-r" | "--require"
                | "--loader"
                | "--import"
                | "--conditions"
                | "--openssl-config"
                | "--icu-data-dir"
        );
        index += if takes_value { 2 } else { 1 };
    }
    None
}

fn derive_bun_workload(
    args: &[String],
    cwd: Option<&Path>,
    anchor: Option<&Path>,
) -> (String, String, bool) {
    let meaningful = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| !arg.starts_with('-'))
        .collect::<Vec<_>>();
    let Some((index, first)) = meaningful.first().copied() else {
        return (
            "interactive".into(),
            "bun interactive".into(),
            anchor.is_some(),
        );
    };

    if matches!(first.as_str(), "run" | "x" | "test" | "build") {
        let subject = args[index + 1..].iter().find(|arg| !arg.starts_with('-'));
        let suffix = subject
            .map(|subject| format!(":{subject}"))
            .unwrap_or_default();
        let label_suffix = subject
            .map(|subject| format!(" {subject}"))
            .unwrap_or_default();
        return (
            format!("command:{first}{suffix}"),
            format!("bun {first}{label_suffix}"),
            anchor.is_some(),
        );
    }

    let (normalized, anchored) = normalize_script(first, cwd, anchor);
    let friendly = friendly_script_name(&normalized);
    (
        format!("script:{normalized}"),
        format!("bun {friendly}"),
        anchored,
    )
}

fn derive_bunx_workload(args: &[String], anchor: Option<&Path>) -> (String, String, bool) {
    let Some(package) = args.iter().find(|arg| !arg.starts_with('-')) else {
        return ("command:x".into(), "bunx".into(), anchor.is_some());
    };
    (
        format!("command:x:{package}"),
        format!("bunx {package}"),
        anchor.is_some(),
    )
}

fn derive_python_workload(
    args: &[String],
    cwd: Option<&Path>,
    anchor: Option<&Path>,
) -> (String, String, bool) {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "-m" => {
                if let Some(module) = args.get(index + 1) {
                    return (
                        format!("module:{module}"),
                        format!("python -m {module}"),
                        anchor.is_some(),
                    );
                }
                break;
            }
            "-c" => {
                let source = args.get(index + 1).map(String::as_str).unwrap_or_default();
                return (
                    format!("inline:{:016x}", stable_hash(source)),
                    "python -c".into(),
                    anchor.is_some(),
                );
            }
            "--" => {
                index += 1;
                break;
            }
            "-W" | "-X" => index += 2,
            _ if arg.starts_with('-') => index += 1,
            _ => break,
        }
    }

    let Some(script) = args.get(index) else {
        return (
            "interactive".into(),
            "python interactive".into(),
            anchor.is_some(),
        );
    };
    let (normalized, anchored) = normalize_script(script, cwd, anchor);
    let friendly = friendly_script_name(&normalized);
    (
        format!("script:{normalized}"),
        format!("python {friendly}"),
        anchored,
    )
}

fn normalize_script(value: &str, cwd: Option<&Path>, anchor: Option<&Path>) -> (String, bool) {
    let trimmed = value.trim_matches('"');
    let path = Path::new(trimmed);
    let full_path = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(cwd) = cwd {
        cwd.join(path)
    } else {
        path.to_path_buf()
    };
    let resolved = fs::canonicalize(&full_path).unwrap_or(full_path);
    let normalized = normalize_path(&resolved);

    if let Some(anchor) = anchor {
        let anchor = normalize_path(anchor);
        let prefix = format!("{anchor}/");
        if let Some(relative) = normalized.strip_prefix(&prefix) {
            return (relative.to_owned(), true);
        }
    }
    if let Some(index) = normalized.find("node_modules/") {
        return (normalized[index..].to_owned(), false);
    }
    if normalized.contains("/uv/cache/") || normalized.contains("/.cache/uv/") {
        if let Some(index) = normalized.rfind("/scripts/") {
            return (format!("uv-tool/{}", &normalized[index + 9..]), false);
        }
        if let Some(index) = normalized.rfind("/bin/") {
            return (format!("uv-tool/{}", &normalized[index + 5..]), false);
        }
    }
    (normalized, false)
}

fn friendly_script_name(script: &str) -> String {
    if let Some(modules) = script.split("node_modules/").nth(1) {
        let mut parts = modules.split('/');
        if let Some(first) = parts.next() {
            if first.starts_with('@')
                && let Some(second) = parts.next()
            {
                return format!("{first}/{second}");
            }
            return first.to_owned();
        }
    }
    Path::new(script)
        .file_name()
        .unwrap_or_else(|| OsStr::new(script))
        .to_string_lossy()
        .into_owned()
}

fn file_stem_lower(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .unwrap_or_else(|| OsStr::new(path))
        .to_string_lossy()
        .to_lowercase()
}

fn stable_hash(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn normalize_path(path: &Path) -> String {
    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                cleaned.pop();
            }
            _ => cleaned.push(component.as_os_str()),
        }
    }
    let normalized = cleaned
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn format_command(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.chars().any(char::is_whitespace) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        thread,
        time::Duration,
    };

    #[test]
    fn runtime_classifier_is_strict_but_handles_versions() {
        let node = if cfg!(windows) {
            "C:\\Program Files\\nodejs\\node.exe"
        } else {
            "/usr/local/bin/node"
        };
        assert_eq!(
            classify_runtime_token(OsStr::new(node)),
            Some(Runtime::Node)
        );
        assert_eq!(
            classify_runtime_token(OsStr::new("python3.13")),
            Some(Runtime::Python)
        );
        assert_eq!(
            classify_runtime_token(OsStr::new("bun.exe")),
            Some(Runtime::Bun)
        );
        assert_eq!(classify_runtime_token(OsStr::new("node_exporter")), None);
        assert_eq!(classify_runtime_token(OsStr::new("python-service")), None);
    }

    #[test]
    fn bun_run_has_a_stable_workload_key() {
        let command = vec!["bun.exe".into(), "run".into(), "dev".into()];
        let (key, label, _) = derive_workload(Runtime::Bun, &command, None, None);
        assert_eq!(key, "command:run:dev");
        assert_eq!(label, "bun run dev");
    }

    #[test]
    fn python_module_has_a_stable_workload_key() {
        let command = vec![
            "python".into(),
            "-u".into(),
            "-m".into(),
            "uvicorn".into(),
            "api:app".into(),
        ];
        let (key, label, _) = derive_workload(Runtime::Python, &command, None, None);
        assert_eq!(key, "module:uvicorn");
        assert_eq!(label, "python -m uvicorn");
    }

    #[test]
    fn node_entrypoint_becomes_project_relative() {
        let entrypoint = if cfg!(windows) {
            "F:\\site\\node_modules\\vite\\bin\\vite.js"
        } else {
            "/srv/site/node_modules/vite/bin/vite.js"
        };
        let command = vec!["node".into(), entrypoint.into()];
        let root = if cfg!(windows) {
            Path::new("F:\\site")
        } else {
            Path::new("/srv/site")
        };
        let (key, label, anchored) =
            derive_workload(Runtime::Node, &command, Some(root), Some(root));
        assert_eq!(key, "script:node_modules/vite/bin/vite.js");
        assert_eq!(label, "node vite");
        assert!(anchored);
    }

    #[test]
    fn global_node_tool_does_not_inherit_the_callers_project() {
        let entrypoint = if cfg!(windows) {
            "C:\\Users\\me\\AppData\\Roaming\\npm\\node_modules\\tool\\cli.js"
        } else {
            "/home/me/.npm-global/lib/node_modules/tool/cli.js"
        };
        let command = vec!["node".into(), entrypoint.into()];
        let root = if cfg!(windows) {
            Path::new("F:\\unrelated-project")
        } else {
            Path::new("/srv/unrelated-project")
        };
        let (key, _, anchored) = derive_workload(Runtime::Node, &command, Some(root), Some(root));
        assert_eq!(key, "script:node_modules/tool/cli.js");
        assert!(!anchored);
    }

    #[test]
    fn uv_cache_hash_is_removed_from_python_tool_identity() {
        let entrypoint = if cfg!(windows) {
            "C:\\Users\\me\\AppData\\Local\\uv\\cache\\archive-v0\\abc123\\Scripts\\android-mcp.exe"
        } else {
            "/home/me/.cache/uv/archive-v0/abc123/bin/android-mcp"
        };
        let command = vec!["python".into(), entrypoint.into()];
        let (key, _, anchored) = derive_workload(Runtime::Python, &command, None, None);
        let expected = if cfg!(windows) {
            "script:uv-tool/android-mcp.exe"
        } else {
            "script:uv-tool/android-mcp"
        };
        assert_eq!(key, expected);
        assert!(!anchored);
    }

    #[test]
    #[ignore = "controlled integration test; requires node on PATH"]
    fn scanner_terminates_only_its_revalidated_child() {
        let mut child = Command::new("node")
            .args(["-e", "setInterval(() => {}, 1000)"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("node must be available for this controlled test");
        let child_pid = child.id();
        thread::sleep(Duration::from_millis(150));

        let mut scanner = ProcessScanner::new();
        let scan = scanner.scan();
        let target = scan
            .processes
            .iter()
            .find(|process| process.pid == child_pid)
            .expect("the scanner should find the exact spawned child")
            .kill_target();
        assert_eq!(target.runtime, Runtime::Node);
        assert_eq!(
            scanner.terminate(&target, true),
            TerminationOutcome::ForceRequested
        );

        for _ in 0..40 {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        panic!("the controlled child did not exit after termination");
    }
}
