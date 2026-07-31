use std::ffi::OsString;

use crate::error::ToolResult;

use super::media::{self, MediaMode};

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Aac,
    Mp3,
    Wav,
}

pub fn run(mode: Mode, args: Vec<OsString>) -> ToolResult {
    let media_mode = match mode {
        Mode::Aac => MediaMode::Audio,
        Mode::Mp3 => MediaMode::Mp3,
        Mode::Wav => MediaMode::Wav,
    };
    media::run(media_mode, args)
}
