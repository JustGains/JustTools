mod commands;
mod common;
mod deps;
mod error;
mod install;
mod pathing;
mod selector;

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

fn main() {
    let invoked = invoked_name();
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let command = match invoked.as_str() {
        "bunt" => "justbunt",
        "rmbg" => "justrmbg",
        _ => invoked.as_str(),
    };
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
