use std::ffi::OsString;

use crate::error::{ToolError, ToolResult};

pub fn installed_launchers() -> Vec<(&'static str, String)> {
    if !cfg!(windows) {
        return Vec::new();
    }
    ["just", "mkcd", "justmkcd", "claude_", "codex_"]
        .into_iter()
        .map(|command| {
            let script = include_str!("shell/powershell.ps1")
                .replace("\r\n", "\n")
                .replace("__JUSTTOOLS_EXE__", "(Join-Path $PSScriptRoot 'just.exe')");
            (command, format!(
                "# JustTools managed PowerShell launcher v1\n{script}\n\
if ($MyInvocation.ExpectingInput) {{\n    $input | {command} @args\n}} else {{\n    {command} @args\n}}\n"
            ))
        })
        .collect()
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    if args.len() == 1 && matches!(args[0].to_str(), Some("-h" | "--help")) {
        println!(
            "just init — Print shell helpers for mkcd, claude_, and codex_.\n\n\
Usage:\n  just init <powershell|bash|zsh|fish>\n\n\
PowerShell: just init powershell | Out-String | Invoke-Expression\n\
Bash:       eval \"$(just init bash)\"\n\
Zsh:        eval \"$(just init zsh)\"\n\
Fish:       just init fish | source\n\n\
Add the matching line to your shell profile for future sessions.\n\
Windows PowerShell launchers are installed automatically by just install.\n\
Defines mkcd, justmkcd, and just functions. Other just commands pass through.\n\
claude_ forwards to claude --dangerously-skip-permissions.\n\
codex_ forwards to codex --yolo. Both forward all additional arguments.\n\
This command only prints the script; it does not edit your profile."
        );
        return Ok(());
    }
    let shell = match args.as_slice() {
        [shell] => shell.to_str().unwrap_or(""),
        _ => {
            return Err(ToolError::usage(
                "just",
                "init requires one shell: powershell, bash, zsh, or fish",
            ));
        }
    };
    let executable = std::env::current_exe()
        .map_err(|error| ToolError::new("just", format!("cannot locate executable: {error}")))?;
    let executable = executable
        .to_str()
        .ok_or_else(|| ToolError::new("just", "executable path must be valid UTF-8"))?;
    let (template, quoted) = match shell {
        "powershell" => (
            include_str!("shell/powershell.ps1"),
            format!("'{}'", executable.replace('\'', "''")),
        ),
        "bash" | "zsh" => (
            include_str!("shell/posix.sh"),
            format!(
                "'{}'",
                if cfg!(windows) {
                    executable.replace('\\', "/")
                } else {
                    executable.to_owned()
                }
                .replace('\'', "'\"'\"'")
            ),
        ),
        "fish" => (
            include_str!("shell/fish.fish"),
            format!(
                "'{}'",
                executable.replace('\\', "\\\\").replace('\'', "\\'")
            ),
        ),
        _ => {
            return Err(ToolError::usage(
                "just",
                format!("unsupported shell: {shell}; use powershell, bash, zsh, or fish"),
            ));
        }
    };
    print!(
        "{}",
        template
            .replace("\r\n", "\n")
            .replace("__JUSTTOOLS_EXE__", &quoted)
    );
    Ok(())
}
