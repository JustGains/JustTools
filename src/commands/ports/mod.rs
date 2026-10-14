mod app;
mod cache;
mod model;
mod scan;
mod ui;

use std::ffi::OsString;

use anyhow::{Context, Result};

use self::{app::App, cache::HistoryStore, model::ServerInfo, scan::ServerScanner};
use crate::error::{ToolError, ToolResult};

const TOOL: &str = "justports";

#[derive(Default)]
struct Options {
    all: bool,
    snapshot: bool,
    json: bool,
    open: Option<u16>,
    stop: Option<StopRequest>,
    history_path: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StopRequest {
    port: u16,
    pid: u32,
    start_time: u64,
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let bare = args.is_empty();
    let args = args
        .iter()
        .map(|argument| crate::common::os_to_string(TOOL, argument, "argument"))
        .collect::<ToolResult<Vec<_>>>()?;
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        print_help();
        return Ok(());
    }
    if args.len() == 1 && matches!(args[0].as_str(), "-V" | "--version") {
        println!("{TOOL} {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut options = parse(&args)?;
    let preferences = if bare {
        let preferences = crate::preferences::Store::load().map_err(|error| {
            ToolError::new(TOOL, format!("could not load saved defaults: {error:#}"))
        })?;
        options.all = preferences.get(TOOL, "show_all") == Some("true");
        Some(preferences)
    } else {
        None
    };
    run_with(options, preferences).map_err(|error| ToolError::new(TOOL, format!("{error:#}")))
}

fn parse(args: &[String]) -> ToolResult<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-a" | "--all" => options.all = true,
            "--snapshot" => options.snapshot = true,
            "--json" => options.json = true,
            "--open" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| ToolError::usage(TOOL, "--open needs a port"))?;
                let port = value.parse::<u16>().map_err(|_| {
                    ToolError::usage(TOOL, format!("port must be from 1 to 65535: {value}"))
                })?;
                if port == 0 {
                    return Err(ToolError::usage(TOOL, "port must be from 1 to 65535"));
                }
                options.open = Some(port);
            }
            "--stop" => {
                if options.stop.is_some() {
                    return Err(ToolError::usage(TOOL, "--stop may only be given once"));
                }
                let values = args.get(index + 1..index + 4).ok_or_else(|| {
                    ToolError::usage(
                        TOOL,
                        "--stop needs PORT PID START_TIME from a current JSON snapshot",
                    )
                })?;
                let port = values[0]
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| ToolError::usage(TOOL, "--stop port must be from 1 to 65535"))?;
                let pid = values[1]
                    .parse::<u32>()
                    .ok()
                    .filter(|pid| *pid > 4)
                    .ok_or_else(|| ToolError::usage(TOOL, "--stop PID must be greater than 4"))?;
                let start_time = values[2]
                    .parse::<u64>()
                    .ok()
                    .filter(|time| *time > 0)
                    .ok_or_else(|| {
                        ToolError::usage(TOOL, "--stop START_TIME must be a Unix timestamp")
                    })?;
                options.stop = Some(StopRequest {
                    port,
                    pid,
                    start_time,
                });
                index += 3;
            }
            "--history-path" => options.history_path = true,
            argument => {
                return Err(ToolError::usage(
                    TOOL,
                    format!("unknown option: {argument}"),
                ));
            }
        }
        index += 1;
    }
    if options.snapshot && options.json {
        return Err(ToolError::usage(
            TOOL,
            "--snapshot cannot be combined with --json",
        ));
    }
    if options.open.is_some() && (options.snapshot || options.json || options.stop.is_some()) {
        return Err(ToolError::usage(
            TOOL,
            "--open cannot be combined with --snapshot, --json or --stop",
        ));
    }
    if options.stop.is_some() && (options.snapshot || options.json || options.all) {
        return Err(ToolError::usage(
            TOOL,
            "--stop cannot be combined with inventory options",
        ));
    }
    if options.history_path
        && (options.open.is_some()
            || options.stop.is_some()
            || options.snapshot
            || options.json
            || options.all)
    {
        return Err(ToolError::usage(
            TOOL,
            "--history-path cannot be combined with other options",
        ));
    }
    Ok(options)
}

fn run_with(options: Options, preferences: Option<crate::preferences::Store>) -> Result<()> {
    if options.history_path {
        println!("{}", cache::cache_path()?.display());
        return Ok(());
    }
    if let Some(request) = options.stop {
        let mut scanner = ServerScanner::new();
        let servers = scanner.scan()?;
        let server = servers
            .iter()
            .find(|server| {
                server.port == request.port
                    && server.pid == request.pid
                    && server.start_time == request.start_time
            })
            .context("listener identity changed; refresh and retry")?;
        scan::terminate_server(server)?;
        println!(
            "justports: stopped {} on port {} (PID {})",
            server.project_name, server.port, server.pid
        );
        return Ok(());
    }
    if let Some(port) = options.open {
        let mut scanner = ServerScanner::new();
        let servers = scanner.scan()?;
        let mut history = HistoryStore::load()?;
        history.record(&servers)?;
        let server = servers
            .iter()
            .find(|server| server.port == port)
            .with_context(|| format!("no TCP listener is running on port {port}"))?;
        app::open_target(&server.url).context("could not open the default browser")?;
        println!("justports: opened {} ({})", server.url, server.project_name);
        return Ok(());
    }

    if options.snapshot || options.json {
        let mut scanner = ServerScanner::new();
        let servers = scanner.scan()?;
        let mut history = HistoryStore::load()?;
        history.record(&servers)?;
        let visible = visible_servers(&servers, options.all);
        if options.json {
            println!("{}", serde_json::to_string_pretty(&visible)?);
        } else {
            print_snapshot(&visible, options.all);
        }
        return Ok(());
    }

    let scanner = ServerScanner::new();
    let preferences = preferences.unwrap_or(crate::preferences::Store::load()?);
    let mut app = App::new(scanner, options.all, preferences)?;
    ratatui::run(|terminal| app.run(terminal)).context("terminal UI failed")?;
    Ok(())
}

fn visible_servers(servers: &[ServerInfo], all: bool) -> Vec<&ServerInfo> {
    servers
        .iter()
        .filter(|server| all || server.is_dev_server)
        .collect()
}

fn print_snapshot(servers: &[&ServerInfo], all: bool) {
    if servers.is_empty() {
        if all {
            println!("justports: no TCP listeners detected");
        } else {
            println!("justports: no development servers detected (try --all)");
        }
        return;
    }
    println!(
        "{:<4} {:>5}  {:<28} {:<22} {:<14} {:>7} PROCESS",
        "DEV", "PORT", "URL", "PROJECT", "STACK", "PID"
    );
    for server in servers {
        println!(
            "{:<4} {:>5}  {:<28} {:<22} {:<14} {:>7} {}",
            if server.is_dev_server { "yes" } else { "no" },
            server.port,
            truncate(&server.url, 28),
            truncate(&server.project_name, 22),
            truncate(&server.framework, 14),
            server.pid,
            server.process_name,
        );
        if !server.command.is_empty() {
            println!("      command: {}", server.command);
        }
        if let Some(root) = &server.project_root {
            println!("      project: {root}");
        }
    }
}

fn truncate(value: &str, width: usize) -> String {
    let mut characters = value.chars();
    let prefix = characters
        .by_ref()
        .take(width.saturating_sub(1))
        .collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        value.to_owned()
    }
}

fn print_help() {
    println!(
        "\
justports — live development server discovery

Usage:
  justports                    Open the smart interactive server browser
  justports --all              Include every local TCP listener
  justports --snapshot         Print detected development servers once
  justports --snapshot --all   Print every TCP listener once
  justports --json [--all]     Emit machine-readable server details
  justports --open PORT        Open a listener directly in the default browser
  justports --stop PORT PID START_TIME
                               Stop the exact listener from a JSON snapshot
  justports --history-path     Print the persistent history file location
  justports -h, --help         Show this help
  justports -V, --version      Show the version

Aliases:
  just ports                   Short JustTools dispatch

Inside the TUI, press Enter to open or start, K to safely stop the selected
Running Now service, Tab to switch Running Now and Launch Again,
p to open a project folder, / to filter, a to toggle smart/all listeners,
and ? for every shortcut."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_supports_automation_modes() {
        let options = parse(&["--json".into(), "--all".into()]).unwrap();
        assert!(options.json);
        assert!(options.all);
        let options = parse(&["--open".into(), "5173".into()]).unwrap();
        assert_eq!(options.open, Some(5173));
        let options = parse(&[
            "--stop".into(),
            "5173".into(),
            "123".into(),
            "1700000000".into(),
        ])
        .unwrap();
        assert_eq!(
            options.stop,
            Some(StopRequest {
                port: 5173,
                pid: 123,
                start_time: 1700000000
            })
        );
    }

    #[test]
    fn incompatible_modes_are_rejected() {
        assert!(parse(&["--json".into(), "--snapshot".into()]).is_err());
        assert!(parse(&["--open".into(), "3000".into(), "--json".into()]).is_err());
        assert!(
            parse(&[
                "--stop".into(),
                "5173".into(),
                "123".into(),
                "1700000000".into(),
                "--json".into()
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--stop".into(),
                "5173".into(),
                "123".into(),
                "1700000000".into(),
                "--open".into(),
                "5173".into()
            ])
            .is_err()
        );
        assert!(parse(&["--stop".into(), "5173".into(), "123".into(), "0".into()]).is_err());
        assert!(
            parse(&[
                "--stop".into(),
                "5173".into(),
                "123".into(),
                "1700000000".into(),
                "--stop".into(),
                "5174".into(),
                "124".into(),
                "1700000001".into()
            ])
            .is_err()
        );
    }
}
