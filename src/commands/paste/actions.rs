//! Paste-as actions: what the clipboard becomes in the destination folder.
//!
//! Ctrl+V in Explorer uses one saved default per kind of clipboard content;
//! the picker shortcut offers every action that fits the clipboard once.
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use super::{Settings, ai, clipboard::Snapshot, convert, free_path};
use crate::common;

/// A file format the clipboard can be pasted as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Txt,
    Md,
    Html,
    Json,
    Png,
    Jpg,
    Webp,
    Ocr,
    Mp3,
    Mp4,
}

impl Format {
    pub const ALL: [Format; 10] = [
        Format::Txt,
        Format::Md,
        Format::Html,
        Format::Json,
        Format::Png,
        Format::Jpg,
        Format::Webp,
        Format::Ocr,
        Format::Mp3,
        Format::Mp4,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.to_ascii_lowercase().as_str() {
            "txt" | "text" => Format::Txt,
            "md" | "markdown" => Format::Md,
            "html" | "htm" => Format::Html,
            "json" => Format::Json,
            "png" => Format::Png,
            "jpg" | "jpeg" => Format::Jpg,
            "webp" => Format::Webp,
            "ocr" => Format::Ocr,
            "mp3" => Format::Mp3,
            "mp4" => Format::Mp4,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Txt => "txt",
            Format::Md => "md",
            Format::Html => "html",
            Format::Json => "json",
            Format::Png => "png",
            Format::Jpg => "jpg",
            Format::Webp => "webp",
            Format::Ocr => "ocr",
            Format::Mp3 => "mp3",
            Format::Mp4 => "mp4",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Txt => "Plain text (.txt)",
            Format::Md => "Markdown (.md)",
            Format::Html => "HTML (.html)",
            Format::Json => "JSON (.json)",
            Format::Png => "PNG image",
            Format::Jpg => "JPEG image",
            Format::Webp => "WebP image",
            Format::Ocr => "Text from image (.txt)",
            Format::Mp3 => "Transcode to MP3",
            Format::Mp4 => "Transcode to MP4",
        }
    }

    fn text(self) -> bool {
        matches!(self, Format::Txt | Format::Md | Format::Html | Format::Json)
    }

    fn image(self) -> bool {
        matches!(self, Format::Png | Format::Jpg | Format::Webp | Format::Ocr)
    }

    fn media(self) -> bool {
        matches!(self, Format::Mp3 | Format::Mp4)
    }

    /// Whether this format applies to `snapshot`.
    pub fn fits(self, snapshot: &Snapshot) -> bool {
        match snapshot {
            Snapshot::Text(_) | Snapshot::Html { .. } => {
                self.text()
                    || (self.media()
                        && snapshot.text().is_some_and(|t| !super::links(t).is_empty()))
            }
            Snapshot::Image(..) => self.image() && (self != Format::Ocr || cfg!(windows)),
            Snapshot::Files { paths, cut: false } => match self {
                Format::Mp3 => !paths.is_empty() && paths.iter().all(|p| media(p).is_some()),
                Format::Mp4 => !paths.is_empty() && paths.iter().all(|p| media(p) == Some(true)),
                _ => false,
            },
            _ => false,
        }
    }
}

const VIDEO: &[&str] = &[
    "3gp", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "ogv", "ts",
    "webm", "wmv",
];
const AUDIO: &[&str] = &[
    "aac", "aif", "aiff", "alac", "amr", "flac", "m4a", "mp3", "oga", "ogg", "opus", "wav", "wma",
];

/// `Some(true)` for a video file, `Some(false)` for audio, `None` otherwise.
pub fn media(path: &Path) -> Option<bool> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if !path.is_file() {
        return None;
    }
    if VIDEO.contains(&extension.as_str()) {
        Some(true)
    } else if AUDIO.contains(&extension.as_str()) {
        Some(false)
    } else {
        None
    }
}

/// One way to paste the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Leave the paste to Explorer.
    Native,
    /// JustPaste's own handling: images, links, repositories, filtered copies.
    Smart,
    As(Format),
    /// Copied page links saved as Markdown through Cloudflare Browser Run.
    PageMarkdown,
    /// Copied text transformed by the configured AI service.
    Ai {
        name: String,
        prompt: String,
    },
}

/// What Ctrl+V does with one kind of clipboard content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnPaste {
    Act(Action),
    /// Show the picker.
    Ask,
}

impl OnPaste {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "ask" => OnPaste::Ask,
            "off" => OnPaste::Act(Action::Native),
            "auto" => OnPaste::Act(Action::Smart),
            "markdown" => OnPaste::Act(Action::PageMarkdown),
            value => OnPaste::Act(Action::As(Format::parse(value)?)),
        })
    }

    pub fn value(&self) -> &'static str {
        match self {
            OnPaste::Ask => "ask",
            OnPaste::Act(Action::Native) => "off",
            OnPaste::Act(Action::PageMarkdown) => "markdown",
            OnPaste::Act(Action::As(format)) => format.name(),
            OnPaste::Act(_) => "auto",
        }
    }
}

/// The saved Ctrl+V default for each kind of clipboard content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defaults {
    pub text: OnPaste,
    pub image: OnPaste,
    pub links: OnPaste,
    pub media: OnPaste,
}

/// Choices offered for each Ctrl+V default, as `(saved value, label)`.
pub const TEXT_CHOICES: &[(&str, &str)] = &[
    ("off", "Leave to Explorer"),
    ("txt", "Plain text (.txt)"),
    ("md", "Markdown (.md)"),
    ("html", "HTML (.html)"),
    ("json", "JSON (.json)"),
    ("ask", "Ask with the picker"),
];
pub const IMAGE_CHOICES: &[(&str, &str)] = &[
    ("png", "PNG image"),
    ("jpg", "JPEG image"),
    ("webp", "WebP image"),
    #[cfg(windows)]
    ("ocr", "Text from image (.txt)"),
    ("ask", "Ask with the picker"),
];
pub const LINK_CHOICES: &[(&str, &str)] = &[
    ("auto", "Download, clone, or save the page"),
    ("markdown", "Page as Markdown (Cloudflare)"),
    ("mp4", "Download as MP4"),
    ("mp3", "Download as MP3"),
    ("ask", "Ask with the picker"),
];
pub const MEDIA_CHOICES: &[(&str, &str)] = &[
    ("off", "Copy normally"),
    ("mp4", "Transcode to MP4"),
    ("mp3", "Transcode to MP3"),
    ("ask", "Ask with the picker"),
];

impl Defaults {
    pub fn load(store: &crate::preferences::Store, text: bool) -> Self {
        let get = |field: &str, choices: &[(&str, &str)], fallback: &str| {
            let value = store
                .get(super::TOOL, field)
                .filter(|value| choices.iter().any(|(known, _)| known == value))
                .unwrap_or(fallback);
            OnPaste::parse(value).expect("choices are valid defaults")
        };
        Self {
            text: get(
                "text_action",
                TEXT_CHOICES,
                if text { "txt" } else { "off" },
            ),
            image: get("image_action", IMAGE_CHOICES, "png"),
            links: get("link_action", LINK_CHOICES, "auto"),
            media: get("media_action", MEDIA_CHOICES, "off"),
        }
    }

    /// What Ctrl+V does with `snapshot`.
    pub fn choose(&self, snapshot: &Snapshot, filters: bool) -> OnPaste {
        match snapshot {
            Snapshot::Image(..) => self.image.clone(),
            Snapshot::Text(_) | Snapshot::Html { .. } => {
                if snapshot
                    .text()
                    .is_some_and(|text| !super::links(text).is_empty())
                {
                    self.links.clone()
                } else {
                    self.text.clone()
                }
            }
            Snapshot::Files { cut: false, .. } if self.media != OnPaste::Act(Action::Native) => {
                let fits = |format| Format::fits(format, snapshot);
                match &self.media {
                    OnPaste::Act(Action::As(format)) if fits(*format) => self.media.clone(),
                    OnPaste::Ask if fits(Format::Mp3) => OnPaste::Ask,
                    _ if filters => OnPaste::Act(Action::Smart),
                    _ => OnPaste::Act(Action::Native),
                }
            }
            Snapshot::Files { cut: false, .. } if filters => OnPaste::Act(Action::Smart),
            _ => OnPaste::Act(Action::Native),
        }
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            text: OnPaste::Act(Action::Native),
            image: OnPaste::Act(Action::As(Format::Png)),
            links: OnPaste::Act(Action::Smart),
            media: OnPaste::Act(Action::Native),
        }
    }
}

/// Shortcuts the picker can use. One key, never one per action.
pub const PICKER_KEYS: &[(&str, &str)] = &[
    ("ctrl+shift+v", "Ctrl+Shift+V"),
    ("win+shift+v", "Win+Shift+V"),
    ("ctrl+alt+v", "Ctrl+Alt+V"),
    ("off", "Off"),
];

/// Which actions the picker lists, and how it describes the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub key: &'static str,
    pub preview: bool,
    pub image_to_text: bool,
    pub paste_as_file: bool,
    pub transcode: bool,
    pub fix_spelling: bool,
    pub ai_prompt: bool,
    /// Show an AI answer for review before saving it (PowerToys' custom preview).
    pub ai_preview: bool,
    /// Close the AI window when it loses focus.
    pub close_on_blur: bool,
}

impl Default for Picker {
    fn default() -> Self {
        Self {
            key: "ctrl+shift+v",
            preview: true,
            image_to_text: true,
            paste_as_file: true,
            transcode: true,
            fix_spelling: true,
            ai_prompt: true,
            ai_preview: true,
            close_on_blur: false,
        }
    }
}

impl Picker {
    pub fn load(store: &crate::preferences::Store) -> Self {
        let flag = |field: &str| store.get(super::TOOL, field) != Some("false");
        let key = store
            .get(super::TOOL, "picker_key")
            .unwrap_or("ctrl+shift+v");
        Self {
            key: PICKER_KEYS
                .iter()
                .find(|(value, _)| *value == key)
                .map_or("ctrl+shift+v", |(value, _)| value),
            preview: flag("clipboard_preview"),
            image_to_text: flag("show_image_to_text"),
            paste_as_file: flag("show_paste_as_file"),
            transcode: flag("show_transcode"),
            fix_spelling: flag("show_fix_spelling"),
            ai_prompt: flag("show_ai_prompt"),
            ai_preview: flag("ai_preview"),
            close_on_blur: store.get(super::TOOL, "ai_close_on_blur") == Some("true"),
        }
    }
}

/// One picker entry. An AI action with an empty prompt asks for one.
pub struct Entry {
    pub label: String,
    pub action: Action,
}

/// Every action that fits `snapshot`, in picker order.
pub fn entries(snapshot: &Snapshot, settings: &Settings, cloudflare: bool) -> Vec<Entry> {
    let picker = &settings.picker;
    let mut entries = vec![Entry {
        label: "Paste normally".into(),
        action: Action::Native,
    }];
    let text = snapshot.text();
    let links = text.is_some_and(|text| !super::links(text).is_empty());
    let smart = match snapshot {
        Snapshot::Image(..) => None,
        _ if links => Some("Download links / clone repositories"),
        Snapshot::Files { cut: false, .. } if settings.filters() => {
            Some("Copy with JustPaste filters")
        }
        _ => None,
    };
    if let Some(label) = smart {
        entries.push(Entry {
            label: label.into(),
            action: Action::Smart,
        });
    }
    if links && cloudflare {
        entries.push(Entry {
            label: "Page as Markdown (Cloudflare)".into(),
            action: Action::PageMarkdown,
        });
    }
    for format in Format::ALL {
        let shown = match format {
            Format::Ocr => picker.image_to_text,
            Format::Mp3 | Format::Mp4 => picker.transcode,
            _ => picker.paste_as_file,
        };
        if shown && format.fits(snapshot) {
            entries.push(Entry {
                label: format.label().into(),
                action: Action::As(format),
            });
        }
    }
    if text.is_some() && settings.ai.is_some() {
        if picker.fix_spelling {
            entries.push(Entry {
                label: ai::FIX_SPELLING.into(),
                action: Action::Ai {
                    name: ai::FIX_SPELLING.into(),
                    prompt: ai::FIX_SPELLING_PROMPT.into(),
                },
            });
        }
        for custom in &settings.custom {
            entries.push(Entry {
                label: custom.name.clone(),
                action: Action::Ai {
                    name: custom.name.clone(),
                    prompt: custom.prompt.clone(),
                },
            });
        }
        if picker.ai_prompt {
            entries.push(Entry {
                label: "Paste with AI…".into(),
                action: Action::Ai {
                    name: "AI paste".into(),
                    prompt: String::new(),
                },
            });
        }
    }
    entries
}

/// A one-line description of the clipboard for the picker. Never a path.
pub fn preview(snapshot: &Snapshot) -> String {
    match snapshot {
        Snapshot::Image(bytes, format) => {
            image::ImageReader::with_format(std::io::Cursor::new(bytes), *format)
                .into_dimensions()
                .map_or_else(|_| "Image".into(), |(w, h)| format!("Image  {w} × {h}"))
        }
        Snapshot::Files { paths, cut } => format!(
            "{} {}{}",
            paths.len(),
            if paths.len() == 1 { "item" } else { "items" },
            if *cut { " (cut)" } else { "" }
        ),
        Snapshot::Text(text) | Snapshot::Html { text, .. } => {
            let links = super::links(text).len();
            if links > 0 {
                return format!("{links} {}", if links == 1 { "link" } else { "links" });
            }
            let line: String = text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(48)
                .collect();
            let more = text.split_whitespace().map(str::len).sum::<usize>() > line.len();
            format!(
                "{}“{line}{}”",
                if matches!(snapshot, Snapshot::Html { .. }) {
                    "Rich text  "
                } else {
                    "Text  "
                },
                if more { "…" } else { "" }
            )
        }
        Snapshot::Unsupported => "Nothing JustPaste can paste".into(),
    }
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string()
}

/// Publish `bytes` as a new file named `name` (numbered when taken), atomically.
pub fn write_new(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    fs::create_dir_all(directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".justpaste-")
        .tempfile_in(directory)
        .map_err(|e| format!("cannot write in {}: {e}", directory.display()))?;
    temp.write_all(bytes).map_err(|e| e.to_string())?;
    if common::interrupted() {
        return Err("cancelled".into());
    }
    let path = free_path(directory, name);
    temp.persist_noclobber(&path)
        .map_err(|e| format!("cannot save {}: {}", path.display(), e.error))?;
    Ok(path)
}

pub fn decode(bytes: &[u8], format: image::ImageFormat) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| format!("cannot decode clipboard image: {e}"))
}

/// Paste `snapshot` into `directory` as `format`. Returns the result line.
pub fn paste_as(
    format: Format,
    snapshot: Snapshot,
    directory: &Path,
    dry_run: bool,
) -> Result<String, String> {
    if !format.fits(&snapshot) {
        return Err(format!(
            "the clipboard holds {}; {} needs {}",
            describe(&snapshot),
            format.label(),
            if format.text() {
                "text"
            } else if format.image() {
                "an image"
            } else if format == Format::Mp4 {
                "copied video files or links"
            } else {
                "copied audio/video files or links"
            }
        ));
    }
    if format.media() {
        let sources: Vec<OsString> = match snapshot {
            Snapshot::Files { paths, .. } => paths.into_iter().map(OsString::from).collect(),
            snapshot => super::links(snapshot.text().unwrap_or_default())
                .into_iter()
                .map(OsString::from)
                .collect(),
        };
        return transcode(format, sources, directory, dry_run);
    }
    if let Snapshot::Image(bytes, source) = snapshot {
        let image = decode(&bytes, source)?;
        let (width, height) = (image.width(), image.height());
        let (name, data) = match format {
            Format::Ocr => {
                if dry_run {
                    return Ok(format!(
                        "dry run: {width}x{height} image text -> {}",
                        directory
                            .join(format!("Image text {}.txt", stamp()))
                            .display()
                    ));
                }
                crate::progress::stage("Reading text from the image…");
                let text = super::ocr::recognize(&image)?;
                if text.trim().is_empty() {
                    return Err("no text was found in the image".into());
                }
                (format!("Image text {}.txt", stamp()), text.into_bytes())
            }
            _ => {
                let extension = format.name();
                let name = format!("Image {}.{extension}", stamp());
                if dry_run {
                    return Ok(format!(
                        "dry run: {width}x{height} {} -> {}",
                        extension.to_ascii_uppercase(),
                        directory.join(name).display()
                    ));
                }
                let data = if format == Format::Png && source == image::ImageFormat::Png {
                    // Keep a valid PNG's original bytes, color metadata, and alpha.
                    bytes
                } else {
                    encode(&image, format)?
                };
                (name, data)
            }
        };
        let path = write_new(directory, &name, &data)?;
        return Ok(format!("saved {} ({width}x{height})", path.display()));
    }
    let (html, text) = match snapshot {
        Snapshot::Html { html, text } => (Some(html), text),
        Snapshot::Text(text) => (None, text),
        _ => unreachable!("text formats fit only text"),
    };
    let content = match format {
        Format::Txt => text,
        Format::Md => convert::markdown(html.as_deref(), &text)?,
        Format::Html => convert::html(html.as_deref(), &text),
        Format::Json => convert::json(&text),
        _ => unreachable!("text formats only"),
    };
    let name = format!("Text {}.{}", stamp(), format.name());
    if dry_run {
        return Ok(format!(
            "dry run: {} -> {}",
            format.label(),
            directory.join(name).display()
        ));
    }
    let path = write_new(directory, &name, content.as_bytes())?;
    Ok(format!("saved {}", path.display()))
}

fn describe(snapshot: &Snapshot) -> &'static str {
    match snapshot {
        Snapshot::Text(_) | Snapshot::Html { .. } => "text",
        Snapshot::Image(..) => "an image",
        Snapshot::Files { cut: true, .. } => "cut files",
        Snapshot::Files { .. } => "copied files",
        Snapshot::Unsupported => "nothing supported",
    }
}

fn encode(image: &image::DynamicImage, format: Format) -> Result<Vec<u8>, String> {
    let mut output = std::io::Cursor::new(Vec::new());
    match format {
        Format::Jpg => {
            // JPEG has no alpha: flatten onto white rather than exposing hidden pixels.
            let rgba = image.to_rgba8();
            let flat = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
                let [r, g, b, a] = rgba.get_pixel(x, y).0;
                let blend = |c: u8| {
                    ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8
                };
                image::Rgb([blend(r), blend(g), blend(b)])
            });
            jpeg_encoder::Encoder::new(&mut output, 90)
                .encode(
                    flat.as_raw(),
                    flat.width() as u16,
                    flat.height() as u16,
                    jpeg_encoder::ColorType::Rgb,
                )
                .map_err(|e| format!("cannot encode JPEG: {e}"))?;
        }
        Format::Webp => image
            .write_to(&mut output, image::ImageFormat::WebP)
            .map_err(|e| format!("cannot encode WebP: {e}"))?,
        _ => image
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|e| format!("cannot encode PNG: {e}"))?,
    }
    Ok(output.into_inner())
}

/// Transcode copied media files or downloaded links with JustVideo/JustMP3,
/// in a hidden workspace, then publish the results under unused names.
fn transcode(
    format: Format,
    sources: Vec<OsString>,
    directory: &Path,
    dry_run: bool,
) -> Result<String, String> {
    if sources.is_empty() {
        return Err("nothing to transcode".into());
    }
    if dry_run {
        for source in &sources {
            println!(
                "  {} -> {} ({}; source kept, no overwrite)",
                source.to_string_lossy(),
                common::display_path(directory),
                format.label()
            );
        }
        return Ok(format!(
            "dry run: {} item(s) to {}",
            sources.len(),
            format.name()
        ));
    }
    fs::create_dir_all(directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;
    let workspace = tempfile::Builder::new()
        .prefix(".justpaste-")
        .tempdir_in(directory)
        .map_err(|e| format!("cannot write in {}: {e}", directory.display()))?;
    let mode = if format == Format::Mp3 {
        crate::commands::media::MediaMode::Mp3
    } else {
        crate::commands::media::MediaMode::Video
    };
    crate::progress::stage(format!("{}…", format.label()));
    let mut args = sources;
    args.push("--output".into());
    args.push(workspace.path().into());
    crate::commands::media::run(mode, args).map_err(|e| e.message().to_owned())?;
    let mut saved = Vec::new();
    let entries = fs::read_dir(workspace.path()).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_file() || name.starts_with('.') {
            continue;
        }
        let destination = free_path(directory, &name);
        tempfile::TempPath::try_from_path(path)
            .map_err(|e| e.to_string())?
            .persist_noclobber(&destination)
            .map_err(|e| format!("cannot save {}: {e}", destination.display()))?;
        saved.push(destination);
    }
    if saved.is_empty() {
        return Err(format!("{} produced no file", format.label()));
    }
    Ok(match saved.as_slice() {
        [one] => format!("saved {}", one.display()),
        many => format!("saved {} {} files", many.len(), format.name()),
    })
}

/// Transform copied text with AI and save the answer as a new file.
pub fn paste_ai(
    config: &ai::Config,
    name: &str,
    prompt: &str,
    snapshot: &Snapshot,
    directory: &Path,
    extension: Format,
    dry_run: bool,
) -> Result<String, String> {
    let text = snapshot
        .text()
        .ok_or("AI paste needs copied text; copy text and try again")?;
    let file = ai_name(name, extension);
    if dry_run {
        return Ok(format!(
            "dry run: {} with {} -> {} (no request made)",
            name,
            config.describe(),
            directory.join(file).display()
        ));
    }
    let answer = ai::transform(config, prompt, text)?;
    save_ai(directory, name, extension, &answer)
}

pub fn ai_name(name: &str, extension: Format) -> String {
    let stem = common::tidy_stem(name);
    let stem = if stem.is_empty() {
        "AI paste".into()
    } else {
        stem
    };
    let extension = if extension.text() {
        extension.name()
    } else {
        "txt"
    };
    format!("{stem} {}.{extension}", stamp())
}

pub fn save_ai(
    directory: &Path,
    name: &str,
    extension: Format,
    answer: &str,
) -> Result<String, String> {
    let path = write_new(directory, &ai_name(name, extension), answer.as_bytes())?;
    Ok(format!("saved {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_fn(4, 3, |x, _| {
            image::Rgba([200, 10, 10, if x == 0 { 0 } else { 255 }])
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        encoded.into_inner()
    }

    #[test]
    fn images_paste_as_jpeg_and_webp_with_their_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        for format in [Format::Jpg, Format::Webp, Format::Png] {
            let snapshot = Snapshot::Image(png(), image::ImageFormat::Png);
            paste_as(format, snapshot, directory.path(), false).unwrap();
        }
        let mut found: Vec<(String, (u32, u32))> = fs::read_dir(directory.path())
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                let extension = path.extension().unwrap().to_string_lossy().into_owned();
                (extension, image::image_dimensions(&path).unwrap())
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("jpg".into(), (4, 3)),
                ("png".into(), (4, 3)),
                ("webp".into(), (4, 3))
            ]
        );
        // The transparent column becomes white in JPEG, never the hidden red.
        let jpeg = fs::read_dir(directory.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().unwrap() == "jpg")
            .unwrap();
        let pixel = image::open(jpeg).unwrap().to_rgb8().get_pixel(0, 1).0;
        assert!(pixel.iter().all(|c| *c > 200), "{pixel:?}");
        let webp = fs::read_dir(directory.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().unwrap() == "webp")
            .unwrap();
        assert_eq!(
            image::open(webp).unwrap().to_rgba8().get_pixel(0, 0).0[3],
            0
        );
    }

    #[test]
    fn text_formats_never_overwrite_and_reject_the_wrong_clipboard() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = || Snapshot::Html {
            html: "<h1>Hi</h1><p>a,b</p>".into(),
            text: "Hi\na,b".into(),
        };
        for format in [
            Format::Txt,
            Format::Md,
            Format::Html,
            Format::Json,
            Format::Md,
        ] {
            paste_as(format, snapshot(), directory.path(), false).unwrap();
        }
        let names: Vec<String> = fs::read_dir(directory.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 5, "{names:?}");
        assert_eq!(names.iter().filter(|n| n.ends_with(".md")).count(), 2);
        let markdown = names.iter().find(|n| n.ends_with(".md")).unwrap();
        assert!(
            fs::read_to_string(directory.path().join(markdown))
                .unwrap()
                .starts_with("# Hi")
        );
        assert!(paste_as(Format::Png, snapshot(), directory.path(), false).is_err());
        assert!(
            paste_as(
                Format::Txt,
                Snapshot::Image(png(), image::ImageFormat::Png),
                directory.path(),
                false
            )
            .is_err()
        );
        paste_as(Format::Json, snapshot(), directory.path(), true).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 5);
    }

    #[test]
    fn ctrl_v_defaults_follow_the_clipboard_content() {
        let defaults = Defaults {
            text: OnPaste::Act(Action::As(Format::Md)),
            image: OnPaste::Ask,
            links: OnPaste::Act(Action::Smart),
            media: OnPaste::Act(Action::As(Format::Mp4)),
        };
        assert_eq!(
            defaults.choose(&Snapshot::Text("hello".into()), false),
            defaults.text
        );
        assert_eq!(
            defaults.choose(&Snapshot::Text("see https://a.test/x".into()), false),
            defaults.links
        );
        assert_eq!(
            defaults.choose(&Snapshot::Image(png(), image::ImageFormat::Png), false),
            OnPaste::Ask
        );
        let directory = tempfile::tempdir().unwrap();
        let clip = directory.path().join("clip.mov");
        let note = directory.path().join("note.txt");
        fs::write(&clip, b"x").unwrap();
        fs::write(&note, b"x").unwrap();
        let files = |paths: Vec<PathBuf>, cut| Snapshot::Files { paths, cut };
        assert_eq!(
            defaults.choose(&files(vec![clip.clone()], false), false),
            defaults.media
        );
        // Mixed selections and cuts stay Explorer's unless filters copy them.
        assert_eq!(
            defaults.choose(&files(vec![clip.clone(), note.clone()], false), false),
            OnPaste::Act(Action::Native)
        );
        assert_eq!(
            defaults.choose(&files(vec![clip.clone(), note], false), true),
            OnPaste::Act(Action::Smart)
        );
        assert_eq!(
            defaults.choose(&files(vec![clip], true), true),
            OnPaste::Act(Action::Native)
        );
        assert_eq!(
            Defaults::default().choose(&Snapshot::Text("hello".into()), false),
            OnPaste::Act(Action::Native)
        );
    }

    #[test]
    fn picker_lists_only_actions_that_fit_and_are_shown() {
        let mut settings = Settings::default();
        let labels = |settings: &Settings, snapshot: &Snapshot| -> Vec<String> {
            entries(snapshot, settings, false)
                .into_iter()
                .map(|e| e.label)
                .collect()
        };
        let text = Snapshot::Text("hello".into());
        assert_eq!(
            labels(&settings, &text),
            [
                "Paste normally",
                "Plain text (.txt)",
                "Markdown (.md)",
                "HTML (.html)",
                "JSON (.json)"
            ]
        );
        settings.ai = Some(ai::Config::default());
        settings.custom = ai::custom_actions("Summarize: Summarize it\n");
        settings.picker.paste_as_file = false;
        assert_eq!(
            labels(&settings, &text),
            [
                "Paste normally",
                ai::FIX_SPELLING,
                "Summarize",
                "Paste with AI…"
            ]
        );
        let image = Snapshot::Image(png(), image::ImageFormat::Png);
        settings.picker.paste_as_file = true;
        assert!(
            labels(&settings, &image)
                .iter()
                .all(|l| !l.contains("AI") && l != ai::FIX_SPELLING)
        );
        assert!(labels(&settings, &image).contains(&"WebP image".to_owned()));
        assert!(preview(&image).contains("4 × 3"));
        assert_eq!(
            preview(&Snapshot::Text("a https://x.test b".into())),
            "1 link"
        );
    }
}
