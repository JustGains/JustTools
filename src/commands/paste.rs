//! `justpaste` — turn a link into a file in the current folder.
//!
//! A link that answers with a file (an image, a video, a PDF, an archive) is
//! saved as that file. A link that answers with a web page is handed to
//! yt-dlp, which knows YouTube, TikTok, and thousands of other sites; when
//! yt-dlp finds nothing, the media the page itself declares is saved instead.
//! Nothing is ever overwritten: a taken name gets " (2)", " (3)", and so on.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ureq::ResponseExt;

use crate::commands::download;
use crate::common;
use crate::deps;
use crate::error::{ToolError, ToolResult};

const TOOL: &str = "justpaste";
/// Many hosts refuse clients that do not look like a browser.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
/// Enough of a page to find the media it declares in its head.
const MAX_PAGE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 150;

struct Options {
    output: Option<PathBuf>,
    clipboard: bool,
    playlist: bool,
    dry_run: bool,
    inputs: Vec<String>,
    help: bool,
}

fn help() {
    println!(
        r#"justpaste — Download whatever a link points at into this folder.

Usage:
  justpaste [options] [url ...]
  justpaste --clipboard

A link to a file (image, video, audio, PDF, archive, anything) is saved as that
file, named as the server names it. A link to a page is downloaded with yt-dlp,
which handles YouTube, TikTok, and most other video and social sites, at the
highest resolution, as an MP4. When yt-dlp finds nothing, the video or image
the page itself declares is saved instead.

Files land in the current folder, or in --output. Nothing is overwritten: a
taken name becomes "name (2).ext". Partial downloads never appear under their
final name. Several links can be given, and --clipboard takes every link in
the copied text.

Options:
  -c, --clipboard     Use the links in the clipboard
  -o, --output DIR    Save into DIR instead of the current folder
      --playlist      Download every entry when a link is a playlist
  -n, --dry-run       Show what would be fetched without any network access
  -h, --help          Show this help

Page links need yt-dlp and FFmpeg, which are downloaded automatically when
missing; direct file links need nothing. yt-dlp is updated whenever twelve
hours have passed since its last update.

Run justpaste with no arguments to open the interactive launcher. Its Headless
footer shows the equivalent direct command; explicit arguments and pipes bypass
the UI."#
    );
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        output: None,
        clipboard: false,
        playlist: false,
        dry_run: false,
        inputs: Vec::new(),
        help: false,
    };
    let mut index = 0;
    while index < args.len() {
        let argument = common::os_to_string(TOOL, &args[index], "argument")?;
        match argument.as_str() {
            "-h" | "--help" => options.help = true,
            "-c" | "--clipboard" => options.clipboard = true,
            "--playlist" => options.playlist = true,
            "-n" | "--dry-run" => options.dry_run = true,
            "-o" | "--output" => {
                index += 1;
                let value = args
                    .get(index)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| ToolError::usage(TOOL, "--output needs a value"))?;
                options.output = Some(PathBuf::from(value));
            }
            _ if argument.starts_with("--output=") => {
                options.output = Some(PathBuf::from(argument.trim_start_matches("--output=")));
            }
            _ if argument.starts_with('-') && argument != "-" => {
                return Err(ToolError::usage(
                    TOOL,
                    format!("unknown option: {argument}"),
                ));
            }
            _ => options.inputs.push(argument),
        }
        index += 1;
    }
    Ok(options)
}

/// Every link in a piece of text, in order, without repeats.
///
/// Copied text often wraps a link in quotes, brackets, or a trailing full
/// stop, so those are trimmed; a bare `www.` address is taken as HTTPS.
fn links(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for token in text.split(|character: char| {
        character.is_whitespace() || matches!(character, '<' | '>' | '"' | '\'' | '`')
    }) {
        let token = token
            .trim_start_matches(['(', '[', '{'])
            .trim_end_matches(['.', ',', ';', '!', ')', ']', '}']);
        let lower = token.to_ascii_lowercase();
        let link = if lower.starts_with("http://") || lower.starts_with("https://") {
            token.to_owned()
        } else if lower.starts_with("www.") && token.len() > 4 {
            format!("https://{token}")
        } else {
            continue;
        };
        if !found.contains(&link) {
            found.push(link);
        }
    }
    found
}

#[cfg(windows)]
fn clipboard_text() -> Result<String, String> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    const UNICODE_TEXT: u32 = 13;

    // Another program may hold the clipboard for a moment while it copies.
    let mut opened = false;
    for _ in 0..10 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    if !opened {
        return Err("the clipboard is in use by another program".into());
    }
    let mut text = String::new();
    unsafe {
        let handle = GetClipboardData(UNICODE_TEXT);
        if !handle.is_null() {
            let data = GlobalLock(handle) as *const u16;
            if !data.is_null() {
                let mut length = 0;
                while *data.add(length) != 0 {
                    length += 1;
                }
                text = String::from_utf16_lossy(std::slice::from_raw_parts(data, length));
                GlobalUnlock(handle);
            }
        }
        CloseClipboard();
    }
    Ok(text)
}

#[cfg(not(windows))]
fn clipboard_text() -> Result<String, String> {
    let readers: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };
    for (program, args) in readers {
        if let Ok(output) = std::process::Command::new(program).args(*args).output()
            && output.status.success()
        {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    Err("no clipboard reader was found (install wl-clipboard, xclip, or xsel)".into())
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |offset: usize| {
            bytes
                .get(index + offset)
                .and_then(|byte| (*byte as char).to_digit(16))
        };
        match (bytes[index], hex(1), hex(2)) {
            (b'%', Some(high), Some(low)) => {
                decoded.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Make a server-supplied name safe to create on every platform.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    let (stem, extension) = match cleaned.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && extension.len() <= 10 => {
            (stem.to_owned(), format!(".{extension}"))
        }
        _ => (cleaned, String::new()),
    };
    let stem: String = stem.chars().take(MAX_NAME_CHARS).collect();
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "lpt1", "lpt2", "lpt3",
    ];
    if reserved.contains(&stem.to_ascii_lowercase().as_str()) {
        format!("_{stem}{extension}")
    } else {
        format!("{stem}{extension}")
    }
}

/// The file name a `Content-Disposition` header asks for.
fn disposition_name(header: &str) -> Option<String> {
    let parameter = |key: &str| {
        header.split(';').map(str::trim).find_map(|part| {
            let (name, value) = part.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case(key)
                .then(|| value.trim().trim_matches('"').to_owned())
        })
    };
    if let Some(encoded) = parameter("filename*") {
        // RFC 5987: charset'language'percent-encoded-name
        let value = encoded.rsplit('\'').next().unwrap_or(&encoded);
        return Some(percent_decode(value));
    }
    parameter("filename")
}

fn extension_for(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/tiff" => "tif",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/json" => "json",
        "application/gzip" => "gz",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/markdown" => "md",
        _ => return None,
    })
}

/// The media type without its parameters, in lowercase.
fn media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn is_page(content_type: &str) -> bool {
    matches!(
        media_type(content_type).as_str(),
        "text/html" | "application/xhtml+xml"
    )
}

/// Choose the saved name: what the server asks for, else the last segment of
/// the address, else the host, with an extension from the content type when
/// the name has none.
fn file_name(url: &str, disposition: Option<&str>, content_type: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let after_scheme = without_query
        .split_once("://")
        .map_or(without_query, |(_, rest)| rest);
    let (host, path) = after_scheme.split_once('/').unwrap_or((after_scheme, ""));
    let segment = path.rsplit('/').find(|segment| !segment.is_empty());
    let mut name = disposition
        .and_then(disposition_name)
        .or_else(|| segment.map(percent_decode))
        .map(|name| sanitize(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| sanitize(host.split(':').next().unwrap_or("download")));
    if name.is_empty() {
        name = "download".into();
    }
    let has_extension = name
        .rsplit_once('.')
        .is_some_and(|(stem, extension)| !stem.is_empty() && !extension.is_empty());
    // A host name's dots are not an extension.
    let named_by_host = disposition.is_none() && segment.is_none();
    if (!has_extension || named_by_host)
        && let Some(extension) = extension_for(&media_type(content_type))
    {
        name = format!("{name}.{extension}");
    }
    name
}

/// A name in `directory` that is not taken, numbering like a file manager.
fn free_path(directory: &Path, name: &str) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    (2_u32..)
        .map(|number| directory.join(format!("{stem} ({number}){extension}")))
        .find(|path| !path.exists())
        .expect("an unused number always exists")
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name) {
        let start = search + found;
        let rest = tag[start + name.len()..].trim_start();
        let preceded = start == 0
            || lower[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if preceded && let Some(value) = rest.strip_prefix('=') {
            let value = value.trim_start();
            let quote = value.chars().next()?;
            return if quote == '"' || quote == '\'' {
                value[1..]
                    .split_once(quote)
                    .map(|(inner, _)| inner.to_owned())
            } else {
                Some(
                    value
                        .split(|character: char| character.is_whitespace() || character == '>')
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                )
            };
        }
        search = start + name.len();
    }
    None
}

/// The media a page declares for link previews, video before image.
fn declared_media(html: &str, page: &str) -> Vec<String> {
    const KEYS: [&str; 7] = [
        "og:video:secure_url",
        "og:video:url",
        "og:video",
        "twitter:player:stream",
        "og:image:secure_url",
        "og:image",
        "twitter:image",
    ];
    let mut found: Vec<(usize, String)> = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut search = 0;
    while let Some(offset) = lower[search..].find("<meta") {
        let start = search + offset;
        let end = lower[start..]
            .find('>')
            .map_or(html.len(), |end| start + end);
        let tag = &html[start..end];
        search = end;
        let key = attribute(tag, "property").or_else(|| attribute(tag, "name"));
        let (Some(key), Some(content)) = (key, attribute(tag, "content")) else {
            continue;
        };
        let Some(rank) = KEYS
            .iter()
            .position(|known| key.eq_ignore_ascii_case(known))
        else {
            continue;
        };
        let content = content.replace("&amp;", "&");
        let link = if content.starts_with("//") {
            format!("https:{content}")
        } else if content.starts_with('/') {
            let origin: String = page.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
            format!("{origin}{content}")
        } else {
            content
        };
        if (link.starts_with("http://") || link.starts_with("https://"))
            && !found.iter().any(|(_, known)| *known == link)
        {
            found.push((rank, link));
        }
    }
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, link)| link).collect()
}

/// What a link answered with.
enum Answer {
    Saved(PathBuf, u64),
    /// A web page, with as much of it as the head is likely to need.
    Page {
        html: String,
        url: String,
    },
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(45)))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

/// Fetch a link, saving it when it is a file and returning it when it is a
/// page. The body streams into a hidden temporary file beside its
/// destination, so an interrupted download leaves nothing behind.
fn retrieve(agent: &ureq::Agent, url: &str, directory: &Path) -> Result<Answer, String> {
    let mut response = agent.get(url).call().map_err(|error| error.to_string())?;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let content_type = header("content-type").unwrap_or_default();
    let disposition = header("content-disposition");
    let final_url = response.get_uri().to_string();
    if is_page(&content_type) && disposition.is_none() {
        let html = response
            .body_mut()
            .with_config()
            .limit(MAX_PAGE_BYTES)
            .read_to_string()
            .unwrap_or_default();
        return Ok(Answer::Page {
            html,
            url: final_url,
        });
    }
    let name = file_name(&final_url, disposition.as_deref(), &content_type);
    let mut temporary = tempfile::Builder::new()
        .prefix(".justpaste-")
        .suffix(".part")
        .tempfile_in(directory)
        .map_err(|error| format!("cannot write in {}: {error}", directory.display()))?;
    let mut reader = response.body_mut().as_reader();
    let mut buffer = vec![0_u8; 256 * 1024];
    let mut written = 0_u64;
    loop {
        if common::interrupted() {
            return Err("interrupted".into());
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("the download stopped: {error}"))?;
        if count == 0 {
            break;
        }
        temporary
            .write_all(&buffer[..count])
            .map_err(|error| format!("cannot write the download: {error}"))?;
        written += count as u64;
    }
    if written == 0 {
        return Err("the server sent an empty file".into());
    }
    temporary
        .flush()
        .map_err(|error| format!("cannot write the download: {error}"))?;
    let destination = free_path(directory, &name);
    temporary
        .persist_noclobber(&destination)
        .map_err(|error| format!("cannot save {}: {}", destination.display(), error.error))?;
    Ok(Answer::Saved(destination, written))
}

/// yt-dlp and FFmpeg, resolved once and only when a page link needs them.
struct Downloader {
    resolved: Option<Result<(PathBuf, PathBuf), String>>,
}

impl Downloader {
    fn get(&mut self) -> Result<(PathBuf, PathBuf), String> {
        self.resolved
            .get_or_insert_with(|| {
                let downloader =
                    download::executable(TOOL).map_err(|error| error.message().to_owned())?;
                let ffmpeg = deps::require_override(TOOL, "FFMPEG_BIN", "ffmpeg")
                    .map_err(|error| error.message().to_owned())?;
                download::update(TOOL, &downloader);
                Ok((downloader, ffmpeg))
            })
            .clone()
    }
}

/// Download a page link with yt-dlp into a hidden folder inside the
/// destination, then move the results out under unused names.
fn with_downloader(
    downloader: &mut Downloader,
    url: &str,
    playlist: bool,
    directory: &Path,
) -> Result<Vec<(PathBuf, u64)>, String> {
    let (executable, ffmpeg) = downloader.get()?;
    let workspace = tempfile::Builder::new()
        .prefix(".justpaste-")
        .tempdir_in(directory)
        .map_err(|error| format!("cannot write in {}: {error}", directory.display()))?;
    let fetched = download::fetch(
        TOOL,
        download::Kind::Original,
        &executable,
        &ffmpeg,
        &[url.to_owned()],
        playlist,
        workspace.path(),
    )
    .map_err(|error| error.message().to_owned())?;
    let mut saved = Vec::new();
    for file in fetched {
        let name = file
            .file
            .file_name()
            .map(|name| sanitize(&name.to_string_lossy()))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "download".into());
        let destination = free_path(directory, &name);
        let size = fs::metadata(&file.file).map_or(0, |metadata| metadata.len());
        fs::rename(&file.file, &destination)
            .map_err(|error| format!("cannot save {}: {error}", destination.display()))?;
        saved.push((destination, size));
    }
    Ok(saved)
}

/// Bring one link into `directory` by whichever route works.
fn paste(
    agent: &ureq::Agent,
    downloader: &mut Downloader,
    url: &str,
    playlist: bool,
    directory: &Path,
) -> Result<Vec<(PathBuf, u64)>, String> {
    let page = match retrieve(agent, url, directory) {
        Ok(Answer::Saved(path, size)) => return Ok(vec![(path, size)]),
        Ok(Answer::Page { html, url }) => Some((html, url)),
        // Sites that refuse a plain request often still work through yt-dlp.
        Err(_) if !common::interrupted() => None,
        Err(error) => return Err(error),
    };
    let reason = match with_downloader(downloader, url, playlist, directory) {
        Ok(saved) => return Ok(saved),
        Err(reason) => reason,
    };
    if let Some((html, page_url)) = &page {
        let candidates = declared_media(html, page_url);
        if !candidates.is_empty() {
            println!("{TOOL}: yt-dlp found nothing; trying the media the page declares");
        }
        for candidate in candidates {
            if let Ok(Answer::Saved(path, size)) = retrieve(agent, &candidate, directory) {
                return Ok(vec![(path, size)]);
            }
        }
    }
    Err(match page {
        Some(_) => format!("nothing downloadable was found on that page ({reason})"),
        None => format!("the link could not be fetched ({reason})"),
    })
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    common::init_signals();
    let options = parse(args)?;
    if options.help {
        help();
        return Ok(());
    }
    let mut urls = links(&options.inputs.join("\n"));
    if urls.len() < options.inputs.len() {
        return Err(ToolError::usage(
            TOOL,
            "every input must be an http(s) link",
        ));
    }
    if options.clipboard {
        let text = clipboard_text().map_err(|message| ToolError::new(TOOL, message))?;
        let copied = links(&text);
        if copied.is_empty() {
            return Err(ToolError::new(
                TOOL,
                "the clipboard holds no http(s) link; copy a link and try again",
            ));
        }
        for link in copied {
            if !urls.contains(&link) {
                urls.push(link);
            }
        }
    } else if urls.is_empty() && !common::stdin_is_terminal() {
        urls = links(&common::read_stdin()?);
    }
    if urls.is_empty() {
        return Err(ToolError::usage(
            TOOL,
            "give a link, or use --clipboard to paste the copied one",
        ));
    }
    let directory = match &options.output {
        Some(output) => output.clone(),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    if options.dry_run {
        println!("{TOOL}: dry run — {} link(s)", urls.len());
        for url in &urls {
            println!(
                "  {url} -> {} (the file itself, or yt-dlp for a page)",
                common::display_path(&directory)
            );
        }
        return Ok(());
    }
    fs::create_dir_all(&directory).map_err(|error| {
        ToolError::new(
            TOOL,
            format!("cannot create {}: {error}", directory.display()),
        )
    })?;
    download::refresh(TOOL);
    let agent = agent();
    let mut downloader = Downloader { resolved: None };
    let mut saved = 0;
    let mut failed = 0;
    for url in &urls {
        if common::interrupted() {
            break;
        }
        println!("{TOOL}: {url}");
        match paste(&agent, &mut downloader, url, options.playlist, &directory) {
            Ok(files) => {
                for (path, size) in files {
                    saved += 1;
                    println!(
                        "  saved   {}  {}",
                        common::display_path(&path),
                        common::format_bytes(size)
                    );
                }
            }
            Err(reason) => {
                failed += 1;
                eprintln!("  failed  {url}: {reason}");
            }
        }
    }
    println!(
        "{TOOL}: {saved} saved{}",
        if failed > 0 {
            format!(", {failed} failed")
        } else {
            String::new()
        }
    );
    if failed > 0 {
        Err(ToolError::new(TOOL, "some links could not be downloaded"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_found_in_copied_text_without_their_wrapping() {
        assert_eq!(
            links(
                "Watch (https://youtu.be/abc123), then <https://example.test/a.jpg>.\nwww.example.test/page https://youtu.be/abc123"
            ),
            [
                "https://youtu.be/abc123",
                "https://example.test/a.jpg",
                "https://www.example.test/page",
            ]
        );
        assert!(links("no links here, ftp://example.test/file").is_empty());
    }

    #[test]
    fn names_come_from_the_server_then_the_address_then_the_host() {
        assert_eq!(
            file_name(
                "https://example.test/a/b/Photo%20One.JPG?size=large",
                None,
                "image/jpeg"
            ),
            "Photo One.JPG"
        );
        assert_eq!(
            file_name(
                "https://example.test/download?id=7",
                Some("attachment; filename=\"report: final?.pdf\""),
                "application/pdf"
            ),
            "report_ final_.pdf"
        );
        assert_eq!(
            file_name(
                "https://example.test/get",
                Some("attachment; filename*=UTF-8''na%C3%AFve%20file.txt"),
                "text/plain"
            ),
            "naïve file.txt"
        );
        assert_eq!(
            file_name("https://cdn.example.test/image/12345", None, "image/webp"),
            "12345.webp"
        );
        assert_eq!(
            file_name("https://example.test/", None, "image/png"),
            "example.test.png"
        );
        assert_eq!(
            file_name("https://example.test/blob", None, "application/x-unknown"),
            "blob"
        );
        assert_eq!(sanitize("con.txt"), "_con.txt");
        assert_eq!(sanitize("trailing. "), "trailing");
    }

    #[test]
    fn taken_names_are_numbered_and_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            free_path(directory.path(), "clip.mp4"),
            directory.path().join("clip.mp4")
        );
        fs::write(directory.path().join("clip.mp4"), b"first").unwrap();
        fs::write(directory.path().join("clip (2).mp4"), b"second").unwrap();
        assert_eq!(
            free_path(directory.path(), "clip.mp4"),
            directory.path().join("clip (3).mp4")
        );
        fs::write(directory.path().join("README"), b"x").unwrap();
        assert_eq!(
            free_path(directory.path(), "README"),
            directory.path().join("README (2)")
        );
    }

    #[test]
    fn pages_are_recognized_and_their_declared_media_ranks_video_first() {
        assert!(is_page("text/html; charset=utf-8"));
        assert!(!is_page("image/jpeg"));
        assert!(!is_page(""));
        let html = r#"<html><head>
            <meta property="og:image" content="//cdn.example.test/cover.jpg?a=1&amp;b=2">
            <META content='/media/clip.mp4' property='og:video'>
            <meta name="twitter:image" content="https://cdn.example.test/cover.jpg?a=1&amp;b=2">
            <meta name="description" content="https://example.test/not-media">
        </head></html>"#;
        assert_eq!(
            declared_media(html, "https://example.test/post/1"),
            [
                "https://example.test/media/clip.mp4",
                "https://cdn.example.test/cover.jpg?a=1&b=2",
            ]
        );
    }

    #[test]
    fn options_parse_and_reject_unknown_switches() {
        let options = parse(vec![
            "--clipboard".into(),
            "-o".into(),
            "downloads".into(),
            "--playlist".into(),
            "https://example.test/a".into(),
        ])
        .unwrap();
        assert!(options.clipboard && options.playlist);
        assert_eq!(options.output, Some(PathBuf::from("downloads")));
        assert_eq!(options.inputs, ["https://example.test/a"]);
        assert!(parse(vec!["--replace".into()]).is_err());
    }
}
