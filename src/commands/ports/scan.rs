use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs,
    net::IpAddr,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState, get_sockets_info};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, get_current_pid};

use super::model::{LaunchRecipe, ServerInfo};

#[derive(Clone, Debug)]
struct ProjectMetadata {
    root: PathBuf,
    name: String,
    package_source: Option<String>,
    launch: Option<LaunchRecipe>,
    framework_hint: Option<String>,
}

#[derive(Clone, Debug)]
struct ListenerGroup {
    port: u16,
    pid: u32,
    addresses: Vec<IpAddr>,
}

pub struct ServerScanner {
    system: System,
    project_cache: HashMap<PathBuf, Option<ProjectMetadata>>,
}

impl ServerScanner {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            project_cache: HashMap::new(),
        }
    }

    pub fn scan(&mut self) -> Result<Vec<ServerInfo>> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );

        let mut groups = listener_groups()?;
        groups.sort_by_key(|listener| (listener.port, listener.pid));
        let mut servers = groups
            .into_iter()
            .map(|listener| self.enrich(listener))
            .collect::<Vec<_>>();
        demote_ephemeral_companions(&mut servers);
        Ok(servers)
    }

    fn enrich(&mut self, listener: ListenerGroup) -> ServerInfo {
        let process = (listener.pid > 0)
            .then(|| self.system.process(Pid::from_u32(listener.pid)))
            .flatten();
        let process_name = process
            .map(|process| process.name().to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into());
        let cwd_path = process
            .and_then(sysinfo::Process::cwd)
            .map(Path::to_path_buf);
        let command_args = process
            .map(|process| {
                process
                    .cmd()
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let executable = process
            .and_then(sysinfo::Process::exe)
            .map(Path::to_path_buf);
        let run_time_seconds = process.map(sysinfo::Process::run_time).unwrap_or_default();
        let start_time = process
            .map(sysinfo::Process::start_time)
            .unwrap_or_default();
        let memory_bytes = process.map(sysinfo::Process::memory).unwrap_or_default();
        let project = cwd_path.as_deref().and_then(|cwd| self.project_for(cwd));
        let command = format_command(&redact_command_args(&command_args));
        let framework = detect_framework(
            &process_name,
            &command,
            project
                .as_ref()
                .and_then(|project| project.package_source.as_deref()),
            project
                .as_ref()
                .and_then(|project| project.framework_hint.as_deref()),
        );
        let (is_dev_server, detection_reason) = classify_dev_server(
            listener.port,
            &process_name,
            &command,
            project.as_ref(),
            &framework,
        );
        let project_root = project
            .as_ref()
            .map(|project| normalize_path(&project.root));
        let project_name = project
            .as_ref()
            .map(|project| project.name.clone())
            .or_else(|| {
                cwd_path
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| process_name.clone());
        let url = browser_url(listener.port, &listener.addresses, &command, &framework);
        let launch = project
            .as_ref()
            .and_then(|project| project.launch.clone())
            .or_else(|| observed_launch(&command_args, cwd_path.as_deref()));

        ServerInfo {
            port: listener.port,
            pid: listener.pid,
            url,
            addresses: listener.addresses.iter().map(ToString::to_string).collect(),
            project_name,
            project_root,
            framework,
            process_name,
            command,
            cwd: cwd_path.as_deref().map(normalize_path),
            executable: executable.as_deref().map(normalize_path),
            run_time_seconds,
            start_time,
            memory_bytes,
            is_dev_server,
            detection_reason,
            launch,
        }
    }

    fn project_for(&mut self, cwd: &Path) -> Option<ProjectMetadata> {
        if let Some(cached) = self.project_cache.get(cwd) {
            return cached.clone();
        }
        let project = discover_project(cwd);
        self.project_cache
            .insert(cwd.to_path_buf(), project.clone());
        project
    }
}

pub fn terminate_server(server: &ServerInfo) -> Result<()> {
    if server.pid <= 4 || server.pid == std::process::id() {
        bail!("refusing to stop protected PID {}", server.pid);
    }
    let system = System::new_all();
    let current_pid = get_current_pid()
        .map_err(|error| anyhow::anyhow!("could not identify the JustPorts process: {error}"))?;
    let current_user = system
        .process(current_pid)
        .and_then(sysinfo::Process::user_id)
        .context("could not identify the current user safely")?;
    let process = system
        .process(Pid::from_u32(server.pid))
        .with_context(|| format!("PID {} ended before it could be stopped", server.pid))?;
    if process.user_id() != Some(current_user) {
        bail!(
            "refusing to stop PID {} because it is not owned by the current user",
            server.pid
        );
    }
    if process.start_time() != server.start_time
        || process.name().to_string_lossy() != server.process_name
    {
        bail!(
            "refusing to stop PID {} because its process identity changed; refresh and retry",
            server.pid
        );
    }
    if !still_owns_listener(server.pid, server.port)? {
        bail!(
            "refusing to stop PID {} because it no longer owns port {}; refresh and retry",
            server.pid,
            server.port
        );
    }
    if !process.kill() {
        bail!(
            "could not stop {} (PID {}); check permissions",
            server.process_name,
            server.pid
        );
    }
    Ok(())
}

fn still_owns_listener(pid: u32, port: u16) -> Result<bool> {
    let sockets = get_sockets_info(
        AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
        ProtocolFlags::TCP,
    )
    .context("could not revalidate local TCP listeners")?;
    Ok(sockets.into_iter().any(|socket| {
        let ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
            return false;
        };
        tcp.state == TcpState::Listen
            && tcp.local_port == port
            && socket.associated_pids.contains(&pid)
    }))
}

fn listener_groups() -> Result<Vec<ListenerGroup>> {
    let sockets = get_sockets_info(
        AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
        ProtocolFlags::TCP,
    )
    .context("could not inspect local TCP listeners")?;
    let mut grouped: BTreeMap<(u16, u32), Vec<IpAddr>> = BTreeMap::new();
    for socket in sockets {
        let ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
            continue;
        };
        if tcp.state != TcpState::Listen || tcp.local_port == 0 {
            continue;
        }
        let pids = if socket.associated_pids.is_empty() {
            vec![0]
        } else {
            socket.associated_pids
        };
        for pid in pids {
            let addresses = grouped.entry((tcp.local_port, pid)).or_default();
            if !addresses.contains(&tcp.local_addr) {
                addresses.push(tcp.local_addr);
            }
        }
    }
    Ok(grouped
        .into_iter()
        .map(|((port, pid), mut addresses)| {
            addresses.sort();
            ListenerGroup {
                port,
                pid,
                addresses,
            }
        })
        .collect())
}

fn discover_project(cwd: &Path) -> Option<ProjectMetadata> {
    let mut current = Some(cwd);
    for _ in 0..20 {
        let directory = current?;
        if let Some(project) = read_project(directory) {
            return Some(project);
        }
        current = directory.parent();
    }
    None
}

fn read_project(directory: &Path) -> Option<ProjectMetadata> {
    if let Some(project_file) = find_project_file(directory, &["csproj", "fsproj", "vbproj"]) {
        let name = project_file
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| directory_name(directory));
        return Some(ProjectMetadata {
            root: directory.to_path_buf(),
            name,
            package_source: None,
            launch: Some(LaunchRecipe {
                label: "dotnet watch run".into(),
                program: "dotnet".into(),
                args: vec!["watch".into(), "run".into()],
                cwd: normalize_path(directory),
            }),
            framework_hint: Some(".NET".into()),
        });
    }

    let package_json = directory.join("package.json");
    if let Some(source) = read_small_file(&package_json)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&source)
    {
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| directory_name(directory));
        return Some(ProjectMetadata {
            root: directory.to_path_buf(),
            name,
            launch: package_launch(directory, &value),
            package_source: Some(source),
            framework_hint: None,
        });
    }

    let cargo_toml = directory.join("Cargo.toml");
    if let Some(source) = read_small_file(&cargo_toml)
        && let Ok(value) = toml::from_str::<toml::Value>(&source)
    {
        let name = value
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| directory_name(directory));
        return Some(ProjectMetadata {
            root: directory.to_path_buf(),
            name,
            package_source: None,
            launch: Some(LaunchRecipe {
                label: "cargo run".into(),
                program: "cargo".into(),
                args: vec!["run".into()],
                cwd: normalize_path(directory),
            }),
            framework_hint: Some("Rust".into()),
        });
    }

    let pyproject = directory.join("pyproject.toml");
    if let Some(source) = read_small_file(&pyproject)
        && let Ok(value) = toml::from_str::<toml::Value>(&source)
    {
        let name = value
            .get("project")
            .and_then(|project| project.get("name"))
            .or_else(|| {
                value
                    .get("tool")
                    .and_then(|tool| tool.get("poetry"))
                    .and_then(|poetry| poetry.get("name"))
            })
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| directory_name(directory));
        return Some(ProjectMetadata {
            root: directory.to_path_buf(),
            name,
            package_source: None,
            launch: python_launch(directory),
            framework_hint: Some("Python".into()),
        });
    }

    let markers = [
        "go.mod",
        "Gemfile",
        "composer.json",
        "requirements.txt",
        "setup.py",
        "setup.cfg",
        "Pipfile",
        "bun.lock",
        "bun.lockb",
        "pnpm-lock.yaml",
        "yarn.lock",
        ".git",
    ];
    markers
        .iter()
        .any(|marker| directory.join(marker).exists())
        .then(|| ProjectMetadata {
            root: directory.to_path_buf(),
            name: directory_name(directory),
            package_source: None,
            launch: common_launch(directory),
            framework_hint: marker_framework(directory),
        })
}

fn package_launch(directory: &Path, package: &serde_json::Value) -> Option<LaunchRecipe> {
    let scripts = package.get("scripts")?.as_object()?;
    let preferred = ["dev", "start", "serve", "web"];
    let script = preferred
        .iter()
        .find(|name| scripts.contains_key(**name))
        .map(|name| (*name).to_owned())
        .or_else(|| scripts.keys().next().cloned())?;
    let program = package_manager(directory);
    Some(LaunchRecipe {
        label: format!("{program} run {script}"),
        program: program.into(),
        args: vec!["run".into(), script],
        cwd: normalize_path(directory),
    })
}

fn package_manager(directory: &Path) -> &'static str {
    for ancestor in directory.ancestors().take(8) {
        if ancestor.join("bun.lock").exists() || ancestor.join("bun.lockb").exists() {
            return "bun";
        }
        if ancestor.join("pnpm-lock.yaml").exists() {
            return "pnpm";
        }
        if ancestor.join("yarn.lock").exists() {
            return "yarn";
        }
        if ancestor.join("package-lock.json").exists() {
            return "npm";
        }
    }
    "npm"
}

fn python_launch(directory: &Path) -> Option<LaunchRecipe> {
    let candidates = [
        ("manage.py", vec!["manage.py", "runserver"]),
        ("app.py", vec!["app.py"]),
        ("main.py", vec!["main.py"]),
    ];
    candidates.iter().find_map(|(file, args)| {
        directory.join(file).is_file().then(|| LaunchRecipe {
            label: format!("python {}", args.join(" ")),
            program: "python".into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: normalize_path(directory),
        })
    })
}

fn common_launch(directory: &Path) -> Option<LaunchRecipe> {
    if directory.join("go.mod").exists() {
        return Some(LaunchRecipe {
            label: "go run .".into(),
            program: "go".into(),
            args: vec!["run".into(), ".".into()],
            cwd: normalize_path(directory),
        });
    }
    if directory.join("composer.json").exists() {
        return Some(LaunchRecipe {
            label: "php -S localhost:8000".into(),
            program: "php".into(),
            args: vec!["-S".into(), "localhost:8000".into()],
            cwd: normalize_path(directory),
        });
    }
    None
}

fn observed_launch(args: &[String], cwd: Option<&Path>) -> Option<LaunchRecipe> {
    if !safe_to_cache_command(args) {
        return None;
    }
    let program = args.first()?.clone();
    let cwd = cwd?;
    Some(LaunchRecipe {
        label: format!("previous: {}", file_stem(&program)),
        program,
        args: args[1..].to_vec(),
        cwd: normalize_path(cwd),
    })
}

fn safe_to_cache_command(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }
    let mut redact_next = false;
    for arg in args {
        let lower = arg.to_lowercase();
        if redact_next || sensitive_assignment(&lower) {
            return false;
        }
        redact_next = sensitive_flag(&lower);
        if (lower.starts_with("http://") || lower.starts_with("https://"))
            && (lower.contains('?') || lower.contains('@'))
        {
            return false;
        }
        if arg.len() > 96 && !Path::new(arg).exists() {
            return false;
        }
    }
    !redact_next
}

fn redact_command_args(args: &[String]) -> Vec<String> {
    let mut redacted = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        let lower = arg.to_lowercase();
        if redact_next {
            redacted.push("[redacted]".into());
            redact_next = false;
            continue;
        }
        if sensitive_assignment(&lower) {
            let name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
            redacted.push(format!("{name}=[redacted]"));
            continue;
        }
        redact_next = sensitive_flag(&lower);
        redacted.push(arg.clone());
    }
    redacted
}

fn sensitive_flag(value: &str) -> bool {
    matches!(
        value.trim_start_matches('-'),
        "token"
            | "api-key"
            | "apikey"
            | "password"
            | "passwd"
            | "secret"
            | "client-secret"
            | "access-token"
            | "auth-token"
            | "credential"
    )
}

fn sensitive_assignment(value: &str) -> bool {
    let Some((name, _)) = value.split_once('=') else {
        return false;
    };
    let name = name.trim_start_matches('-').replace('_', "-");
    sensitive_flag(&name)
        || name.ends_with("-token")
        || name.ends_with("-password")
        || name.ends_with("-secret")
        || name.ends_with("-api-key")
}

fn classify_dev_server(
    port: u16,
    process_name: &str,
    command: &str,
    project: Option<&ProjectMetadata>,
    framework: &str,
) -> (bool, String) {
    let haystack = format!("{process_name} {command}").to_lowercase();
    let explicit_terms = [
        "vite",
        "next dev",
        "next-server",
        "astro dev",
        "webpack",
        "parcel",
        "react-scripts start",
        "ng serve",
        "nuxt dev",
        "remix dev",
        "expo start",
        "metro",
        "storybook",
        "nodemon",
        "tsx watch",
        "uvicorn",
        "hypercorn",
        "flask run",
        "manage.py runserver",
        "rails server",
        "php -s",
        "dotnet watch",
        "cargo run",
        "air",
    ];
    if let Some(term) = explicit_terms.iter().find(|term| haystack.contains(**term)) {
        return (true, format!("matched {term}"));
    }

    let runtime = file_stem(process_name);
    let development_runtime = matches!(
        runtime.as_str(),
        "node"
            | "nodejs"
            | "bun"
            | "deno"
            | "python"
            | "python3"
            | "pythonw"
            | "ruby"
            | "php"
            | "dotnet"
            | "java"
    );
    if project.is_some() && development_runtime && port >= 1024 {
        return (true, format!("{runtime} listener in a project"));
    }
    if project.is_some() && framework != "Web server" && port >= 1024 {
        return (true, format!("{framework} listener in a project"));
    }
    (false, "not recognized as a development server".into())
}

fn detect_framework(
    process_name: &str,
    command: &str,
    package_source: Option<&str>,
    project_hint: Option<&str>,
) -> String {
    let process_text = format!("{process_name} {command}").to_lowercase();
    let direct_candidates = [
        ("dotnet", ".NET"),
        ("expo", "Expo / Metro"),
        ("metro", "Expo / Metro"),
        ("uvicorn", "Uvicorn"),
        ("fastapi", "FastAPI"),
        ("flask", "Flask"),
        ("django", "Django"),
        ("rails", "Rails"),
        ("spring", "Spring"),
        ("cargo", "Rust"),
        ("php", "PHP"),
        ("next", "Next.js"),
        ("astro", "Astro"),
        ("vite", "Vite"),
        ("nuxt", "Nuxt"),
        ("webpack", "Webpack"),
        ("parcel", "Parcel"),
        ("storybook", "Storybook"),
    ];
    if let Some(label) = direct_candidates
        .iter()
        .find_map(|(needle, label)| process_text.contains(needle).then_some(*label))
    {
        return label.to_owned();
    }
    let text = package_source.unwrap_or_default().to_lowercase();
    let candidates = [
        ("next", "Next.js"),
        ("astro", "Astro"),
        ("vite", "Vite"),
        ("nuxt", "Nuxt"),
        ("webpack", "Webpack"),
        ("parcel", "Parcel"),
        ("storybook", "Storybook"),
        ("expo", "Expo / Metro"),
    ];
    candidates
        .iter()
        .find_map(|(needle, label)| text.contains(needle).then_some((*label).to_owned()))
        .or_else(|| project_hint.map(str::to_owned))
        .unwrap_or_else(|| "Web server".into())
}

fn browser_url(port: u16, addresses: &[IpAddr], command: &str, framework: &str) -> String {
    let lower = command.to_lowercase();
    let scheme = if port == 443
        || port == 8443
        || lower.contains("--https")
        || lower.contains("https://")
        || lower.contains("--ssl")
        || lower.contains("--cert")
        || (port >= 7000 && lower.contains("launch-profile https"))
        || (framework == ".NET" && (7000..8000).contains(&port))
    {
        "https"
    } else {
        "http"
    };
    let host = addresses
        .iter()
        .find(|address| address.is_loopback())
        .or_else(|| addresses.iter().find(|address| !address.is_unspecified()))
        .map(|address| match address {
            IpAddr::V4(address) => address.to_string(),
            IpAddr::V6(address) => format!("[{address}]"),
        })
        .unwrap_or_else(|| "localhost".into());
    format!("{scheme}://{host}:{port}/")
}

fn demote_ephemeral_companions(servers: &mut [ServerInfo]) {
    let mut candidate_ports: HashMap<u32, Vec<u16>> = HashMap::new();
    for server in servers.iter().filter(|server| server.is_dev_server) {
        candidate_ports
            .entry(server.pid)
            .or_default()
            .push(server.port);
    }
    for server in servers {
        let has_stable_companion = candidate_ports
            .get(&server.pid)
            .is_some_and(|ports| ports.iter().any(|port| *port < 49_152));
        let dotnet_watch_internal =
            server.port >= 49_152 && server.command.to_lowercase().contains("dotnet-watch");
        if server.is_dev_server
            && server.port >= 49_152
            && (has_stable_companion || dotnet_watch_internal)
        {
            server.is_dev_server = false;
            server.detection_reason = "ephemeral companion listener for the same process".into();
        }
    }
}

fn marker_framework(directory: &Path) -> Option<String> {
    if directory.join("go.mod").exists() {
        Some("Go".into())
    } else if directory.join("Gemfile").exists() {
        Some("Rails".into())
    } else if directory.join("composer.json").exists() {
        Some("PHP".into())
    } else {
        None
    }
}

fn find_project_file(directory: &Path, extensions: &[&str]) -> Option<PathBuf> {
    fs::read_dir(directory).ok()?.find_map(|entry| {
        let path = entry.ok()?.path();
        path.is_file()
            .then(|| path.extension()?.to_str())
            .flatten()
            .is_some_and(|extension| {
                extensions
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            })
            .then_some(path)
    })
}

fn read_small_file(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    (metadata.is_file() && metadata.len() <= 1_048_576)
        .then(|| fs::read_to_string(path).ok())
        .flatten()
}

fn directory_name(directory: &Path) -> String {
    directory
        .file_name()
        .unwrap_or(directory.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn file_stem(value: &str) -> String {
    Path::new(value)
        .file_stem()
        .unwrap_or_else(|| OsStr::new(value))
        .to_string_lossy()
        .to_lowercase()
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
    cleaned
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_owned()
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

    #[test]
    fn wildcard_listener_becomes_localhost_url() {
        assert_eq!(
            browser_url(4321, &["0.0.0.0".parse().unwrap()], "vite", "Vite"),
            "http://localhost:4321/"
        );
    }

    #[test]
    fn loopback_is_preferred_over_lan_addresses() {
        assert_eq!(
            browser_url(
                3000,
                &[
                    "192.168.1.50".parse().unwrap(),
                    "127.0.0.1".parse().unwrap()
                ],
                "next dev",
                "Next.js"
            ),
            "http://127.0.0.1:3000/"
        );
    }

    #[test]
    fn explicit_dev_commands_are_detected_without_project_metadata() {
        let (is_dev, reason) = classify_dev_server(
            5173,
            "node.exe",
            "node node_modules/vite/bin/vite.js",
            None,
            "Vite",
        );
        assert!(is_dev);
        assert!(reason.contains("vite"));
    }

    #[test]
    fn unrelated_high_port_listener_is_not_a_dev_server() {
        let (is_dev, _) = classify_dev_server(5353, "chrome.exe", "chrome.exe", None, "Web server");
        assert!(!is_dev);
    }

    #[test]
    fn high_dynamic_listener_is_demoted_when_process_has_a_stable_port() {
        let mut servers = vec![test_server(3000), test_server(53_043)];
        demote_ephemeral_companions(&mut servers);
        assert!(servers[0].is_dev_server);
        assert!(!servers[1].is_dev_server);
        assert!(servers[1].detection_reason.contains("ephemeral"));
    }

    fn test_server(port: u16) -> ServerInfo {
        ServerInfo {
            port,
            pid: 42,
            url: format!("http://localhost:{port}/"),
            addresses: vec!["127.0.0.1".into()],
            project_name: "demo".into(),
            project_root: Some("F:/demo".into()),
            framework: "Vite".into(),
            process_name: "node".into(),
            command: "vite".into(),
            cwd: Some("F:/demo".into()),
            executable: None,
            run_time_seconds: 1,
            start_time: 1,
            memory_bytes: 1,
            is_dev_server: true,
            detection_reason: "test".into(),
            launch: None,
        }
    }

    #[test]
    fn package_json_supplies_project_name() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("package.json"),
            r#"{"name":"great-dev-app","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        let project = discover_project(directory.path()).unwrap();
        assert_eq!(project.name, "great-dev-app");
        assert_eq!(
            detect_framework(
                "node",
                "",
                project.package_source.as_deref(),
                project.framework_hint.as_deref()
            ),
            "Vite"
        );
    }

    #[test]
    fn secrets_are_redacted_and_never_cached_as_launch_recipes() {
        let args = vec![
            "node".into(),
            "server.js".into(),
            "--api-key".into(),
            "do-not-store-this".into(),
        ];
        assert!(!safe_to_cache_command(&args));
        assert_eq!(
            format_command(&redact_command_args(&args)),
            "node server.js --api-key [redacted]"
        );
    }
}
