mod app;
mod config;
mod model;
mod process;
mod ui;

use std::{ffi::OsString, thread};

use anyhow::{Context, Result};

use self::app::App;
use self::config::{ConfigStore, config_path};
use self::model::ProcessInfo;
use self::process::ProcessScanner;
use crate::error::{ToolError, ToolResult};

const TOOL: &str = "justbunt";

pub fn run(args: Vec<OsString>) -> ToolResult {
    let args = args
        .iter()
        .map(|argument| crate::common::os_to_string(TOOL, argument, "argument"))
        .collect::<ToolResult<Vec<_>>>()?;

    match args.as_slice() {
        [] => run_tui().map_err(runtime_error),
        [arg] if matches!(arg.as_str(), "-h" | "--help") => {
            print_help();
            Ok(())
        }
        [arg] if matches!(arg.as_str(), "-V" | "--version") => {
            println!("{TOOL} {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        [arg] if arg == "--config-path" => {
            println!("{}", config_path().map_err(runtime_error)?.display());
            Ok(())
        }
        [arg] if arg == "--snapshot" => print_snapshot().map_err(runtime_error),
        _ => Err(ToolError::usage(
            TOOL,
            format!(
                "unknown arguments: {}",
                args.iter()
                    .map(|arg| format!("`{arg}`"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        )),
    }
}

fn runtime_error(error: anyhow::Error) -> ToolError {
    ToolError::new(TOOL, format!("{error:#}"))
}

fn run_tui() -> Result<()> {
    let store = ConfigStore::load()?;
    let scanner = ProcessScanner::new();
    let mut app = App::new(scanner, store);
    ratatui::run(|terminal| app.run(terminal)).context("terminal UI failed")?;
    Ok(())
}

fn print_snapshot() -> Result<()> {
    let store = ConfigStore::load()?;
    let mut scanner = ProcessScanner::new();
    scanner.scan();
    thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    let mut scan = scanner.scan();
    scan.processes
        .sort_by_key(|process| (process.runtime.as_str(), process.pid));

    println!(
        "{:<10} {:<7} {:>7} {:>7} {:>10} {:<24} WORKLOAD",
        "STATE", "RUNTIME", "PID", "CPU", "MEMORY", "PROJECT"
    );
    for process in &scan.processes {
        let state = snapshot_state(process, &scan.processes, &scan.launcher_ancestry, &store);
        println!(
            "{state:<10} {:<7} {:>7} {:>6.1}% {:>10} {:<24} {}",
            process.runtime,
            process.pid,
            process.cpu_percent,
            format_bytes(process.memory_bytes),
            truncate(&process.project_name, 24),
            process.workload_label,
        );
        if !process.command.is_empty() {
            println!("           cmd: {}", process.command);
        }
    }
    Ok(())
}

fn snapshot_state(
    process: &ProcessInfo,
    processes: &[ProcessInfo],
    launcher_ancestry: &std::collections::HashSet<u32>,
    store: &ConfigStore,
) -> &'static str {
    let mut current = process;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current.pid) {
        if launcher_ancestry.contains(&current.pid) {
            return "safety";
        }
        if store.matching_rule(&current.identity).is_some() {
            return "excluded";
        }
        let Some(parent_pid) = current.parent_pid else {
            break;
        };
        let Some(parent) = processes
            .iter()
            .find(|candidate| candidate.pid == parent_pid)
        else {
            break;
        };
        current = parent;
    }
    "target"
}

fn print_help() {
    println!(
        "\
justbunt — smart Node, Bun, and Python process manager

Usage:
  justbunt                 Open the interactive TUI
  justbunt --snapshot      Print one read-only process snapshot
  justbunt --config-path   Print the persistent configuration path
  justbunt -h, --help      Show this help
  justbunt -V, --version   Show the version

Aliases:
  bunt                     Direct installed alias
  just bunt                Short JustTools dispatch

Inside the TUI, press ? for keys and smart-filter examples."
    );
}

fn truncate(value: &str, width: usize) -> String {
    let mut characters = value.chars();
    let truncated = characters
        .by_ref()
        .take(width.saturating_sub(1))
        .collect::<String>();
    if characters.next().is_some() {
        format!("{truncated}…")
    } else {
        value.to_owned()
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.0} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.0} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}
