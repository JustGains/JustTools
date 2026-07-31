use std::ffi::OsString;

use crate::error::ToolResult;

pub fn run(args: Vec<OsString>) -> ToolResult {
    super::core_port::run("justjson", args, justtools_core::json::run)
}
