pub mod common;
pub mod json;
pub mod pdf;
pub mod port;
pub mod qr;
pub mod svg;

use std::ffi::OsString;
use std::process::ExitCode;

/// Runs one core command with arguments supplied by the root multicall binary.
pub fn run_with_args(
    tool: &str,
    args: Vec<OsString>,
    run: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    common::with_cli_args(tool, args, run)
}

/// Consistent binary entry point: concise diagnostics and conventional exit codes.
pub fn main_result(tool: &str, run: impl FnOnce() -> anyhow::Result<()>) -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{tool}: {error:#}");
            ExitCode::FAILURE
        }
    }
}
