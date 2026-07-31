use std::ffi::OsString;

use crate::commands::{self, COMMANDS};
use crate::error::{ToolError, ToolResult};

fn list() {
    let width = COMMANDS
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(4);
    println!("just — run one of the compiled just* tools\n");
    for command in COMMANDS {
        println!(
            "  {:width$}  {}",
            command.name,
            command.description,
            width = width
        );
    }
    if let Ok(directory) = crate::pathing::current_bin_directory()
        && !crate::pathing::contains(&directory)
    {
        println!("\n  Add To Path  add {} to your PATH", directory.display());
    }
    println!("\nrun: just <tool> [args]   (e.g. `just qr hello`, `just help video`)");
}

fn normalized_tool(value: &str) -> String {
    if value.starts_with("just") {
        value.to_ascii_lowercase()
    } else {
        format!("just{}", value.to_ascii_lowercase())
    }
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    if args.len() == 1 && (args[0] == "-V" || args[0] == "--version") {
        println!("just {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "install") {
        return crate::install::run(args.into_iter().skip(1).collect());
    }
    if args
        .first()
        .is_some_and(|arg| arg == "add-to-path" || arg == "--add-to-path")
    {
        if args.len() > 1 {
            return Err(ToolError::usage(
                "just",
                "add-to-path does not take arguments",
            ));
        }
        return crate::pathing::add(&crate::pathing::current_bin_directory()?);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "-h" || arg == "--help" || arg == "list")
    {
        if args.len() > 1 {
            return Err(ToolError::usage(
                "just",
                "--help/list does not take extra arguments",
            ));
        }
        list();
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "help") {
        if args.len() == 1 {
            list();
            return Ok(());
        }
        if args.len() > 2 {
            return Err(ToolError::usage(
                "just",
                "help accepts at most one tool name",
            ));
        }
        let requested = args[1].to_string_lossy();
        return commands::dispatch(&normalized_tool(&requested), vec![OsString::from("--help")]);
    }
    if let Some(first) = args.first() {
        let text = first
            .to_str()
            .ok_or_else(|| ToolError::usage("just", "tool name must be valid UTF-8"))?;
        if text.starts_with('-') {
            return Err(ToolError::usage("just", format!("unknown option: {text}")));
        }
        return commands::dispatch(&normalized_tool(text), args.into_iter().skip(1).collect());
    }
    if !crate::common::stdin_is_terminal() || !crate::common::stdout_is_terminal() {
        list();
        return Ok(());
    }
    let mut names: Vec<String> = COMMANDS
        .iter()
        .map(|command| format!("{:<12} {}", command.name, command.description))
        .collect();
    let path_action = crate::pathing::current_bin_directory()
        .ok()
        .filter(|directory| !crate::pathing::contains(directory));
    if let Some(directory) = &path_action {
        names.push(format!("Add To Path  add {} to PATH", directory.display()));
    }
    let selection = dialoguer::Select::new()
        .with_prompt("just · run which tool?")
        .items(&names)
        .default(0)
        .interact_opt()
        .map_err(|error| ToolError::new("just", error.to_string()))?
        .ok_or_else(|| ToolError::cancelled("just"))?;
    if selection == COMMANDS.len() {
        crate::pathing::add(path_action.as_deref().expect("PATH action is present"))
    } else {
        commands::dispatch(COMMANDS[selection].name, Vec::new())
    }
}
