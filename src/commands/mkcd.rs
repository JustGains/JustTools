use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::error::{ToolError, ToolResult};

const TOOL: &str = "justmkcd";

pub fn run(args: Vec<OsString>) -> ToolResult {
    let mut parents = false;
    let mut print_path = false;
    let mut positional = false;
    let mut directory = None;
    for argument in args {
        if !positional {
            match argument.to_str() {
                Some("-h" | "--help") => {
                    println!(
                        "justmkcd — Create a directory and enter it.\n\n\
Usage:\n  mkcd [options] DIRECTORY\n  just mkcd [options] DIRECTORY\n\n\
Options:\n  -p, --parents  Create missing parents; accept an existing directory\n  \
    --print-path  Create and print the absolute path for headless scripts\n  \
-h, --help     Show this help\n\n\
Accepts exactly one directory. Use -- before a name starting with -.\n\
Bare invocation opens the console UI. Paths are never saved.\n\n\
Windows PowerShell: just install provides automatic launchers; run mkcd directly.\n\
Explicit .exe invocation bypasses the PowerShell launcher.\n\n\
Enable shell integration once per session (add to your shell profile to persist):\n  \
PowerShell: just init powershell | Out-String | Invoke-Expression\n  \
Bash:       eval \"$(just init bash)\"\n  \
Zsh:        eval \"$(just init zsh)\"\n  \
Fish:       just init fish | source\n\n\
A native executable cannot change its parent shell's directory. Without the\n\
integration, this command creates the directory and prints its absolute path."
                    );
                    return Ok(());
                }
                Some("-p" | "--parents") => {
                    parents = true;
                    continue;
                }
                Some("--print-path") => {
                    print_path = true;
                    continue;
                }
                Some("--") => {
                    positional = true;
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(ToolError::usage(TOOL, format!("unknown option: {value}")));
                }
                _ => {}
            }
        }
        if argument.is_empty() {
            return Err(ToolError::usage(TOOL, "directory must not be empty"));
        }
        if directory.replace(PathBuf::from(argument)).is_some() {
            return Err(ToolError::usage(TOOL, "expected exactly one directory"));
        }
    }
    let directory = directory.ok_or_else(|| ToolError::usage(TOOL, "directory is required"))?;
    if directory.to_str().is_none() {
        return Err(ToolError::usage(TOOL, "directory must be valid UTF-8"));
    }
    // The wrapper owns this temporary file. Open it before creating anything so
    // a broken shell handoff cannot leave an unexpected new directory behind.
    let mut result = if print_path {
        None
    } else {
        std::env::var_os("JUSTTOOLS_MKCD_RESULT")
            .map(|path| {
                OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(path)
                    .map_err(|error| {
                        ToolError::new(TOOL, format!("cannot open shell result: {error}"))
                    })
            })
            .transpose()?
    };
    let created = if parents {
        fs::create_dir_all(&directory)
    } else {
        fs::create_dir(&directory)
    };
    created.map_err(|error| {
        ToolError::new(
            TOOL,
            format!("cannot create {}: {error}", directory.display()),
        )
    })?;
    // Resolve after creation so the shell enters the exact directory even when
    // the input traverses a symlink followed by '..'.
    let absolute = directory.canonicalize().map_err(|error| {
        ToolError::new(TOOL, format!("cannot resolve created directory: {error}"))
    })?;
    let path_text = absolute
        .to_str()
        .ok_or_else(|| ToolError::new(TOOL, "resolved directory must be valid UTF-8"))?;
    if let Some(result) = &mut result {
        result
            .write_all(path_text.as_bytes())
            .and_then(|()| result.write_all(&[0]))
            .and_then(|()| result.flush())
            .map_err(|error| ToolError::new(TOOL, format!("cannot write shell result: {error}")))?;
    } else {
        println!("{path_text}");
        if !print_path {
            eprintln!(
                "justmkcd: directory created; enable `just init <shell>` to change your shell directory (see just mkcd --help)."
            );
        }
    }
    Ok(())
}
