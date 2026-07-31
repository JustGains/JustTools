use std::ffi::OsString;

use crate::error::ToolResult;

pub fn run(args: Vec<OsString>) -> ToolResult {
    super::core_port::run("justport", args, justtools_core::port::run)
}
