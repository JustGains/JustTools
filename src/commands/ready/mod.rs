mod catalog;
mod detect;
mod plan;
mod ui;

use std::collections::HashSet;
use std::ffi::OsString;

use serde::Serialize;

use self::catalog::{App, Platform};
use self::detect::Detection;
use crate::error::{ToolError, ToolResult};

const TOOL: &str = "justready";

#[derive(Debug, Default)]
struct Options {
    list: bool,
    json: bool,
    install: Vec<String>,
    recommended: bool,
    dry_run: bool,
    yes: bool,
    help: bool,
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let options = parse(args)?;
    if options.help {
        print_help();
        return Ok(());
    }
    let platform = Platform::current().ok_or_else(|| {
        ToolError::new(
            TOOL,
            "this operating system is not supported; JustReady supports Windows, macOS, and Linux",
        )
    })?;
    let apps = catalog::for_platform(platform);

    if options.list || options.json {
        let detection = scan_with_notice(platform, &apps, options.json);
        if options.json {
            print_json(platform, &apps, &detection)?;
        } else {
            print_list(platform, &apps, &detection);
        }
        return Ok(());
    }

    if options.recommended || !options.install.is_empty() {
        let detection = scan_with_notice(platform, &apps, false);
        let ids = requested_ids(&options, &apps, &detection)?;
        return install_selection(
            platform,
            &apps,
            &ids,
            detection,
            options.dry_run,
            options.yes,
            false,
        );
    }

    if options.dry_run || options.yes {
        return Err(ToolError::usage(
            TOOL,
            "--dry-run/--yes needs --install IDS or --recommended",
        ));
    }
    if !crate::common::stdin_is_terminal() || !crate::common::stdout_is_terminal() {
        return Err(ToolError::usage(
            TOOL,
            "the interactive picker needs a terminal; use --list, --json, --recommended, or --install IDS",
        ));
    }

    let Some(selection) = ui::choose(platform, apps.clone())
        .map_err(|error| ToolError::new(TOOL, format!("terminal UI failed: {error}")))?
    else {
        println!("No changes made.");
        return Ok(());
    };
    install_selection(
        platform,
        &apps,
        &selection.ids,
        selection.detection,
        false,
        true,
        true,
    )
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        let argument = crate::common::os_to_string(TOOL, &args[index], "argument")?;
        match argument.as_str() {
            "-h" | "--help" => options.help = true,
            "--list" => options.list = true,
            "--json" => options.json = true,
            "--recommended" => options.recommended = true,
            "--dry-run" => options.dry_run = true,
            "-y" | "--yes" => options.yes = true,
            "--install" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| ToolError::usage(TOOL, "--install needs an app id list"))?;
                let value = crate::common::os_to_string(TOOL, value, "--install")?;
                add_ids(&mut options.install, &value)?;
            }
            _ if argument.starts_with("--install=") => {
                add_ids(
                    &mut options.install,
                    argument.trim_start_matches("--install="),
                )?;
            }
            _ => {
                return Err(ToolError::usage(
                    TOOL,
                    format!("unknown option: {argument}"),
                ));
            }
        }
        index += 1;
    }

    if options.help && args.len() > 1 {
        return Err(ToolError::usage(TOOL, "--help does not take other options"));
    }
    if (options.list || options.json)
        && (options.recommended || !options.install.is_empty() || options.dry_run || options.yes)
    {
        return Err(ToolError::usage(
            TOOL,
            "--list/--json cannot be combined with installation options",
        ));
    }
    Ok(options)
}

fn add_ids(ids: &mut Vec<String>, value: &str) -> ToolResult {
    let values = value
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(ToolError::usage(TOOL, "--install needs an app id list"));
    }
    ids.extend(values);
    Ok(())
}

fn scan_with_notice(platform: Platform, apps: &[App], quiet: bool) -> Detection {
    if !quiet {
        eprintln!("Checking installed software for {}…", platform.label());
    }
    detect::scan(platform, apps)
}

fn requested_ids(
    options: &Options,
    apps: &[App],
    detection: &Detection,
) -> ToolResult<Vec<String>> {
    let mut ids = Vec::new();
    if options.recommended {
        ids.extend(
            apps.iter()
                .filter(|app| app.recommended && !detection.installed(app.id))
                .map(|app| app.id.to_owned()),
        );
    }
    for requested in &options.install {
        let normalized = normalize_id(requested);
        let alias = match normalized.as_str() {
            "github" | "github-gui" => "github-desktop",
            "gh" => "github-cli",
            "claude" => "claude-code",
            "claude-app" | "claude-desktop-app" => "claude-desktop",
            "openai-codex" => "codex",
            "dbeaver-community" => "dbeaver",
            "share-x" => "sharex",
            _ => normalized.as_str(),
        };
        let Some(app) = apps
            .iter()
            .find(|app| app.id.eq_ignore_ascii_case(alias) || normalize_id(app.name) == normalized)
        else {
            return Err(ToolError::usage(
                TOOL,
                format!(
                    "unknown or unavailable app `{requested}` on this OS; run `justready --list` for valid ids"
                ),
            ));
        };
        ids.push(app.id.to_owned());
    }
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    Ok(ids)
}

fn normalize_id(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

fn install_selection(
    platform: Platform,
    apps: &[App],
    ids: &[String],
    detection: Detection,
    dry_run: bool,
    yes: bool,
    confirmed_in_tui: bool,
) -> ToolResult {
    if ids.is_empty() {
        println!("All requested apps are already installed.");
        return Ok(());
    }
    let install_plan = plan::build(platform, apps, ids, &detection)
        .map_err(|error| ToolError::new(TOOL, error))?;
    if install_plan.actions.is_empty() {
        println!("All requested apps are already installed.");
        return Ok(());
    }
    print_plan(apps, &install_plan);
    if dry_run {
        println!("\nDry run only — no system changes were made.");
        return Ok(());
    }
    if !yes && !confirmed_in_tui {
        let question = format!(
            "Run these {} installation step(s)?",
            install_plan.actions.len()
        );
        if !crate::common::confirm(TOOL, &question)? {
            println!("No changes made.");
            return Ok(());
        }
    }

    let execution = plan::execute(&install_plan).map_err(|error| ToolError::new(TOOL, error))?;
    println!("\nVerifying installed software…");
    let after = detect::scan(platform, apps);
    let verified = install_plan
        .app_ids
        .iter()
        .filter(|id| after.installed(id))
        .count();
    let pending = install_plan.app_ids.len().saturating_sub(verified);
    println!(
        "\nJustReady finished: {verified} verified, {} installer failure(s), {pending} awaiting a new shell or app registration.",
        execution.failed.len()
    );
    if !execution.failed.is_empty() {
        for (label, error) in &execution.failed {
            eprintln!("  failed: {label} ({error})");
        }
        return Err(ToolError::new(
            TOOL,
            format!("{} app installation(s) failed", execution.failed.len()),
        ));
    }
    if pending > 0 {
        println!("Open a new terminal before retrying commands installed into your user PATH.");
    }
    Ok(())
}

fn print_plan(apps: &[App], install_plan: &plan::InstallPlan) {
    let names = plan::names_for_ids(apps, &install_plan.app_ids);
    println!("\nSelected: {}", names.join(", "));
    if !install_plan.dependency_ids.is_empty() {
        println!(
            "Dependencies: {}",
            plan::names_for_ids(apps, &install_plan.dependency_ids).join(", ")
        );
    }
    println!("\nInstallation plan:");
    for (index, line) in plan::preview_lines(install_plan).iter().enumerate() {
        println!("  {}. {line}", index + 1);
    }
}

fn print_list(platform: Platform, apps: &[App], detection: &Detection) {
    println!("JustReady catalog for {}\n", platform.label());
    let mut prior = None;
    for app in apps {
        if prior != Some(app.section) {
            if prior.is_some() {
                println!();
            }
            println!("{}", app.section.label());
            prior = Some(app.section);
        }
        let state = if detection.installed(app.id) {
            "installed"
        } else {
            "available"
        };
        let star = if app.recommended { " ★" } else { "" };
        println!("  {:<18} {:<10} {}{}", app.id, state, app.name, star);
        println!("  {:<18}            {}", "", app.description);
    }
    println!(
        "\n★ recommended · {} apps available for this OS",
        apps.len()
    );
    for warning in &detection.warnings {
        eprintln!("justready: scan note: {warning}");
    }
}

#[derive(Serialize)]
struct JsonCatalog<'a> {
    platform: Platform,
    warnings: &'a [String],
    apps: Vec<JsonApp<'a>>,
}

#[derive(Serialize)]
struct JsonApp<'a> {
    id: &'a str,
    name: &'a str,
    description: &'a str,
    section: &'a str,
    recommended: bool,
    installed: bool,
    installer: &'a str,
    package: Option<&'a str>,
}

fn print_json(platform: Platform, apps: &[App], detection: &Detection) -> ToolResult {
    let report = JsonCatalog {
        platform,
        warnings: &detection.warnings,
        apps: apps
            .iter()
            .map(|app| JsonApp {
                id: app.id,
                name: app.name,
                description: app.description,
                section: app.section.label(),
                recommended: app.recommended,
                installed: detection.installed(app.id),
                installer: app.source.label(),
                package: app.source.package_key(),
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| ToolError::new(TOOL, format!("cannot serialize catalog: {error}")))?;
    println!("{json}");
    Ok(())
}

fn print_help() {
    println!(
        r#"justready — curated cross-platform software setup

Usage:
  justready                              Open the interactive catalog
  justready --list                       List apps available for this OS
  justready --json                       Print the detected catalog as JSON
  justready --recommended [options]      Install missing recommendations
  justready --install ID[,ID...]         Install specific apps by catalog id

Options:
      --recommended  Include every missing recommended app
      --install IDS  Comma-separated ids; repeatable
      --dry-run      Print the complete dependency-aware plan only
  -y, --yes          Confirm the printed plan non-interactively
      --list         Print the sectioned OS-specific catalog
      --json         Print the catalog and installed state as JSON
  -h, --help         Show this help

Examples:
  justready --install github,github-cli,git
  justready --install codex,claude-code,zed --dry-run
  justready --recommended --yes
  just ready --list

The installed-software scan runs in the background inside the TUI. Installed
apps cannot be selected. JustReady adds required package-manager prerequisites,
shows the exact plan, restores the terminal, then runs native installers with
their progress and privilege prompts visible."#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_mutating_list_combinations() {
        let error = parse(vec!["--list".into(), "--yes".into()]).unwrap_err();
        assert!(error.message().contains("cannot be combined"));
    }

    #[test]
    fn comma_separated_ids_are_normalized_later() {
        let options = parse(vec!["--install".into(), "Git, Claude".into()]).unwrap();
        assert_eq!(options.install, ["Git", "Claude"]);
    }

    #[test]
    fn claude_cli_and_desktop_aliases_stay_distinct() {
        let apps = catalog::for_platform(Platform::Windows);
        let detection = Detection::test_with(detect::SystemState::default(), &[]);
        let options = parse(vec![
            "--install".into(),
            "claude,claude-app".into(),
            "--dry-run".into(),
        ])
        .unwrap();
        assert_eq!(
            requested_ids(&options, &apps, &detection).unwrap(),
            ["claude-code", "claude-desktop"]
        );
    }
}
