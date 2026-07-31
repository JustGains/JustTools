use crate::error::ToolResult;
use std::ffi::OsString;
pub fn run(args: Vec<OsString>) -> ToolResult {
    super::media::run(super::media::MediaMode::Video, args)
}
