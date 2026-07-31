use std::ffi::OsString;

use crate::error::{ToolError, ToolResult};

pub mod audio;
pub mod avif;
mod core_port;
pub mod crop;
mod image_ops;
pub mod jpg;
pub mod json;
mod media;
pub mod pdf;
pub mod png;
pub mod port;
pub mod qr;
pub mod resize;
pub mod rmbg;
pub mod svg;
pub mod video;
pub mod webp;
pub mod zip;

#[derive(Clone, Copy, Debug)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "justaudio",
        description: "convert audio or video soundtracks to compact AAC/M4A",
    },
    CommandInfo {
        name: "justavif",
        description: "convert still images to compact AVIF files",
    },
    CommandInfo {
        name: "justcrop",
        description: "trim transparent image borders to their alpha bounds",
    },
    CommandInfo {
        name: "justjpg",
        description: "create optimized progressive JPEG files",
    },
    CommandInfo {
        name: "justjson",
        description: "format, validate, query, or minify JSON",
    },
    CommandInfo {
        name: "justmp3",
        description: "convert audio or video soundtracks to high-quality MP3",
    },
    CommandInfo {
        name: "justpdf",
        description: "inspect, merge, split, extract, or rotate PDFs",
    },
    CommandInfo {
        name: "justpng",
        description: "optimize PNG files quickly",
    },
    CommandInfo {
        name: "justport",
        description: "find what is using a local port",
    },
    CommandInfo {
        name: "justqr",
        description: "generate a ready-to-scan QR code",
    },
    CommandInfo {
        name: "justresize",
        description: "resize still images with safe web-ready defaults",
    },
    CommandInfo {
        name: "justrmbg",
        description: "remove image backgrounds locally with BRIA RMBG-2.0",
    },
    CommandInfo {
        name: "justsvg",
        description: "optimize SVGs with SVGOMG-style conservative defaults",
    },
    CommandInfo {
        name: "justvideo",
        description: "optimize videos as streaming-ready 720p H.264 MP4",
    },
    CommandInfo {
        name: "justwav",
        description: "convert audio or video soundtracks to editing-ready WAV",
    },
    CommandInfo {
        name: "justwebp",
        description: "convert images to compact lossy WebP files",
    },
    CommandInfo {
        name: "justzip",
        description: "archive a Git working tree while honoring every Git ignore rule",
    },
];

pub fn dispatch(command: &str, args: Vec<OsString>) -> ToolResult {
    if args.len() == 1 && args[0] == "--version" {
        println!("{command} {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    match command {
        "justaudio" => audio::run(audio::Mode::Aac, args),
        "justmp3" => audio::run(audio::Mode::Mp3, args),
        "justwav" => audio::run(audio::Mode::Wav, args),
        "justavif" => avif::run(args),
        "justcrop" => crop::run(args),
        "justjpg" => jpg::run(args),
        "justpng" => png::run(args),
        "justvideo" => video::run(args),
        "justwebp" => webp::run(args),
        "justzip" => zip::run(args),
        "justjson" => json::run(args),
        "justpdf" => pdf::run(args),
        "justport" => port::run(args),
        "justqr" => qr::run(args),
        "justresize" => resize::run(args),
        "justrmbg" => rmbg::run(args),
        "justsvg" => svg::run(args),
        _ => Err(ToolError::usage("just", format!("unknown tool: {command}"))),
    }
}
