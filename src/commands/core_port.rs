use std::ffi::OsString;

use crate::error::{ToolError, ToolResult};

pub fn run(
    tool: &'static str,
    args: Vec<OsString>,
    command: impl FnOnce() -> anyhow::Result<()>,
) -> ToolResult {
    justtools_core::run_with_args(tool, args, command).map_err(|error| {
        let message = format!("{error:#}");
        if message == "cancelled" {
            ToolError::cancelled(tool)
        } else if let Some(message) = message.strip_prefix("error: ") {
            ToolError::usage(tool, message.trim())
        } else {
            ToolError::new(tool, message)
        }
    })
}
