mod commands;
mod common;
mod console_status;
mod console_ui;
mod context;
mod deps;
mod error;
mod install;
mod launcher;
mod managed;
mod pathing;
mod preferences;
mod progress;
mod selector;
mod shell;

use std::ffi::OsString;
use std::path::Path;

fn invoked_name() -> String {
    let executable = std::env::args_os()
        .next()
        .unwrap_or_else(|| OsString::from("just"));
    Path::new(&executable)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("just")
        .to_ascii_lowercase()
}

/// JustPaste's tray controls own no console output anyone needs to see.
#[cfg(any(windows, test))]
fn windowless(command: &str, args: &[OsString]) -> bool {
    let args = match command {
        "justpaste" => args,
        "just" | "justtools" if args.first().is_some_and(|tool| tool == "paste") => &args[1..],
        _ => return false,
    };
    matches!(
        args.first().and_then(|arg| arg.to_str()),
        Some("--enable" | "--disable" | "--background" | "--settings" | "--startup")
    )
}

/// The app manifest asks Windows 11 24H2 and later not to allocate a console
/// when Explorer, Start, or sign-in starts this program, so JustPaste's tray
/// never flashes a window. Every other command gets the console it had.
#[cfg(windows)]
fn ensure_console(command: &str, args: &[OsString]) {
    use windows::Win32::System::Console::{
        AllocConsole, GetConsoleWindow, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE,
    };
    let handed = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
        .into_iter()
        .any(|id| unsafe { GetStdHandle(id) }.is_ok_and(|handle| !handle.is_invalid()));
    // A console, hidden or not, or redirected handles mean nothing is missing.
    if handed || !unsafe { GetConsoleWindow() }.is_invalid() || windowless(command, args) {
        return;
    }
    let _ = unsafe { AllocConsole() };
}

fn main() {
    let invoked = invoked_name();
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let command = match invoked.as_str() {
        "bunt" => "justbunt",
        "rmbg" => "justrmbg",
        "mkcd" => "justmkcd",
        _ => invoked.as_str(),
    };
    #[cfg(windows)]
    ensure_console(command, &args);
    let result = if command == "just" || command == "justtools" {
        selector::run(args)
    } else {
        commands::dispatch(command, args)
    };

    if let Err(error) = result {
        eprintln!("{}: {}", error.tool(), error.message());
        if error.exit_code() == 2 {
            eprintln!("Try '{} --help'.", error.tool());
        }
        std::process::exit(error.exit_code());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_justpaste_tray_controls_start_without_a_console() {
        for (command, values) in [
            ("justpaste", &["--enable"][..]),
            ("justpaste", &["--background"]),
            ("justpaste", &["--settings"]),
            ("justpaste", &["--startup", "on"]),
            ("just", &["paste", "--enable"]),
            ("just", &["paste", "--disable"]),
        ] {
            assert!(windowless(command, &args(values)), "{command} {values:?}");
        }
        for (command, values) in [
            ("justpaste", &[][..]),
            ("justpaste", &["--status"]),
            ("justpaste", &["--diagnose"]),
            ("justpaste", &["--clipboard"]),
            ("just", &["paste"]),
            ("just", &["webp", "--enable"]),
            ("justwebp", &["--settings"]),
            ("just", &[]),
        ] {
            assert!(!windowless(command, &args(values)), "{command} {values:?}");
        }
    }
}
