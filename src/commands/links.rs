use std::ffi::OsString;

use crate::error::ToolResult;

pub fn run(args: Vec<OsString>) -> ToolResult {
    super::core_port::run("justlinks", args, justtools_core::links::run)
}
