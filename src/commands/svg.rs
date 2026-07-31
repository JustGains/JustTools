use std::ffi::OsString;

use crate::error::ToolResult;

pub fn run(args: Vec<OsString>) -> ToolResult {
    super::core_port::run("justsvg", args, justtools_core::svg::run)
}
