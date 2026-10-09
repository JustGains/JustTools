//! `justpaste` — paste files or clone a repository into the current folder.
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
mod clipboard;
mod copy;
mod git;
#[cfg(windows)]
mod windows;

#[derive(Clone, Debug, Default)]
struct Settings {
    excludes: Vec<String>,
    gitignore: bool,
    text: bool,
}

impl Settings {
    fn saved() -> Result<Self, String> {
        let store = crate::preferences::Store::load().map_err(|e| e.to_string())?;
        let mut excludes = exclusions(store.get(TOOL, "excludes").unwrap_or(""))?;
        if store.get(TOOL, "ignore_node_modules") == Some("true") {
            excludes.push("node_modules".into());
        }
        Ok(Self {
            excludes,
            gitignore: store.get(TOOL, "gitignore") == Some("true"),
            text: store.get(TOOL, "text") == Some("true"),
        })
    }
    #[cfg(windows)]
    fn filters(&self) -> bool {
        self.gitignore || !self.excludes.is_empty()
    }
}

fn exclusions(value: &str) -> Result<Vec<String>, String> {
    value.split(';').map(str::trim).filter(|s| !s.is_empty()).map(|value| {
        let value = value.replace('\\', "/").trim_matches('/').to_owned();
        if value.is_empty() || value.contains([':', '*', '?']) || value.split('/').any(|p| matches!(p, "." | ".." | "")) {
            return Err("ignore names must be names or relative paths, separated by semicolons; no wildcards or ..".into());
        }
        Ok(value)
    }).collect()
}
/// Many hosts refuse clients that do not look like a browser.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
/// Enough of a page to find the media it declares in its head.
const MAX_PAGE_BYTES: u64 = 4 * 1024 * 1024;

struct Options {
    output: Option<PathBuf>,
    clipboard: bool,
    playlist: bool,
    dry_run: bool,
    inputs: Vec<String>,
    help: bool,
    settings: Settings,
    copies: Vec<PathBuf>,
}

fn help() {
    println!(
        r#"justpaste — Paste files or clone a Git repository into this folder.

Usage:
  justpaste [options] [url ...]
  justpaste --clipboard

A link to a file (image, video, audio, PDF, archive, anything) is saved as that
file, named as the server names it. A link to a page is downloaded with yt-dlp,
which handles YouTube, TikTok, and most other video and social sites, at the
highest resolution, as an MP4 named after its title. When yt-dlp finds nothing,
the video or image the page itself declares is saved instead.

Repository homepages (GitHub, GitLab, Bitbucket, Codeberg, Gitea, SourceHut,
Azure Repos), http(s) clone links ending in .git, and SSH/git links use git
clone. The complete checkout lands in a repository-named subfolder, including
its history and origin. Git must be installed; existing credentials are used.

Names are tidied before saving: emoji, symbols, links, and hashtags are
dropped, and a long name is cut at a word near 80 characters.

Files land in the current folder, or in --output. Nothing is overwritten: a
taken name becomes "name (2).ext" or "repo (2)". Failed or cancelled clones
leave no partial checkout. Several links can be given, and --clipboard takes every link in
the copied text.

Options:
  -c, --clipboard     Use the links in the clipboard
      --copy PATH     Copy a file/folder (repeatable); combine with --dry-run
      --text          Save plain clipboard or piped text as a UTF-8 .txt file
      --exclude NAMES Skip names/relative paths in copied trees (semicolon-separated)
      --ignore-node-modules  Skip node_modules in copied trees
      --respect-gitignore    Honor .gitignore and .git/info/exclude when copying
      --saved-settings      Explicitly use saved copy filters and text setting
  -o, --output DIR    Save into DIR instead of the current folder
      --playlist      Download every entry when a link is a playlist
  -n, --dry-run       Show what would be fetched without any network access
  -h, --help          Show this help

Windows, macOS, Linux: clipboard images save as PNG; copied files are copied,
never moved. Cut payloads are refused. Files are never overwritten. macOS uses
its native pasteboard; Linux needs wl-paste (Wayland) or xclip (X11), with xsel
as a text-only fallback. URLs, --copy, and piped text also work without a desktop.

Windows-only Explorer Smart Paste controls:
  --enable           Start Explorer Ctrl+V Smart Paste with a tray icon
  --disable          Stop Smart Paste (does not change login startup)
  --status           Show whether Smart Paste is running
  --settings         Open the native Windows settings window
  --diagnose         Inspect Explorer focus and shell connectivity (no clipboard data)
  --startup on|off    Set current-user login startup
Tray settings and the bare launcher save copy filters. Explicit headless paste
uses only supplied flags. Filters do not affect Explorer cut/move operations.

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
        settings: if args.iter().any(|arg| arg == "--saved-settings") {
            Settings::saved().map_err(|e| ToolError::new(TOOL, e))?
        } else {
            Settings::default()
        },
        copies: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let argument = common::os_to_string(TOOL, &args[index], "argument")?;
        match argument.as_str() {
            "-h" | "--help" => options.help = true,
            "-c" | "--clipboard" => options.clipboard = true,
            "--playlist" => options.playlist = true,
            "--saved-settings" => {}
            "--copy" => {
                index += 1;
                let value = args
                    .get(index)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| ToolError::usage(TOOL, "--copy needs a path"))?;
                options.copies.push(PathBuf::from(value));
            }
            "--text" => options.settings.text = true,
            "--respect-gitignore" => options.settings.gitignore = true,
            "--ignore-node-modules" => options.settings.excludes.push("node_modules".into()),
            "--exclude" => {
                let value = common::option_value(TOOL, &args, &mut index, "--exclude")?;
                options
                    .settings
                    .excludes
                    .extend(exclusions(&value).map_err(|e| ToolError::usage(TOOL, e))?);
            }
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
        } else if git::repository(token).is_some() {
            token.to_owned()
        } else {
            continue;
        };
        if !found.contains(&link) {
            found.push(link);
        }
    }
    found
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

/// A server-supplied name as its raw stem and its extension, when the part
/// after the last dot looks like one.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    let name = name.trim().trim_end_matches(['.', ' ']);
    match name.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && (1..=10).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (stem, Some(extension))
        }
        _ => (name, None),
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
/// the address, else the host, tidied into a short plain stem, with an
/// extension from the content type when the name has none.
fn file_name(url: &str, disposition: Option<&str>, content_type: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let after_scheme = without_query
        .split_once("://")
        .map_or(without_query, |(_, rest)| rest);
    let (host, path) = after_scheme.split_once('/').unwrap_or((after_scheme, ""));
    let segment = path.rsplit('/').find(|segment| !segment.is_empty());
    let given = disposition
        .and_then(disposition_name)
        .or_else(|| segment.map(percent_decode))
        .unwrap_or_default();
    let (stem, extension) = split_extension(&given);
    // A name that tidies to nothing (all emoji, say) falls back to the host,
    // whose dots are not an extension.
    let stem = [stem, host.split(':').next().unwrap_or_default()]
        .into_iter()
        .map(common::tidy_stem)
        .find(|stem| !stem.is_empty())
        .unwrap_or_else(|| "download".into());
    match extension.or_else(|| extension_for(&media_type(content_type))) {
        Some(extension) => format!("{stem}.{extension}"),
        None => stem,
    }
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
    crate::progress::stage("Connecting to the link…");
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
    let total = header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0);
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
        crate::progress::report(
            "Downloading file…",
            format!("{} downloaded", common::format_bytes(written)),
            total.map(|n| (written.saturating_mul(100) / n).min(100) as u32),
        );
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
                crate::progress::stage("Preparing yt-dlp…");
                let downloader =
                    download::executable(TOOL).map_err(|error| error.message().to_owned())?;
                crate::progress::stage("Preparing FFmpeg…");
                let ffmpeg = deps::require_override(TOOL, "FFMPEG_BIN", "ffmpeg")
                    .map_err(|error| error.message().to_owned())?;
                download::update(TOOL, &downloader);
                if common::interrupted() {
                    return Err("cancelled".into());
                }
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
        // yt-dlp wrote under a fixed name; the tidied title is the real one.
        let name = match file.file.extension() {
            Some(extension) => format!("{}.{}", file.stem, extension.to_string_lossy()),
            None => file.stem,
        };
        let destination = free_path(directory, &name);
        let size = fs::metadata(&file.file).map_or(0, |metadata| metadata.len());
        // Publish without replacing a file created after free_path was checked.
        tempfile::TempPath::try_from_path(file.file)
            .map_err(|error| error.to_string())?
            .persist_noclobber(&destination)
            .map_err(|error| format!("cannot save {}: {error}", destination.display()))?;
        saved.push((destination, size));
    }
    Ok(saved)
}

/// Bring one link into `directory` by whichever route works.
enum Pasted {
    File(PathBuf, u64),
    Repository(PathBuf),
}

fn paste(
    agent: &ureq::Agent,
    downloader: &mut Downloader,
    url: &str,
    playlist: bool,
    directory: &Path,
) -> Result<Vec<Pasted>, String> {
    if let Some(repository) = git::repository(url) {
        return git::clone(&repository, directory).map(|path| vec![Pasted::Repository(path)]);
    }
    let page = match retrieve(agent, url, directory) {
        Ok(Answer::Saved(path, size)) => return Ok(vec![Pasted::File(path, size)]),
        Ok(Answer::Page { html, url }) => Some((html, url)),
        // Sites that refuse a plain request often still work through yt-dlp.
        Err(_) if !common::interrupted() => None,
        Err(error) => return Err(error),
    };
    let reason = match with_downloader(downloader, url, playlist, directory) {
        Ok(saved) => {
            return Ok(saved
                .into_iter()
                .map(|(path, size)| Pasted::File(path, size))
                .collect());
        }
        Err(reason) => reason,
    };
    if common::interrupted() {
        return Err("cancelled".into());
    }
    if let Some((html, page_url)) = &page {
        let candidates = declared_media(html, page_url);
        if !candidates.is_empty() {
            println!("{TOOL}: yt-dlp found nothing; trying the media the page declares");
        }
        for candidate in candidates {
            if let Ok(Answer::Saved(path, size)) = retrieve(agent, &candidate, directory) {
                return Ok(vec![Pasted::File(path, size)]);
            }
        }
    }
    Err(match page {
        Some(_) => format!("nothing downloadable was found on that page ({reason})"),
        None => format!("the link could not be fetched ({reason})"),
    })
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    if args.first().is_some_and(|a| {
        matches!(
            a.to_str(),
            Some(
                "--enable"
                    | "--disable"
                    | "--status"
                    | "--background"
                    | "--startup"
                    | "--diagnose"
                    | "--settings"
            )
        )
    }) {
        #[cfg(windows)]
        return windows::control(&args).map_err(|e| ToolError::new(TOOL, e));
        #[cfg(not(windows))]
        return Err(ToolError::usage(
            TOOL,
            "Explorer Smart Paste is Windows-only",
        ));
    }
    common::init_signals();
    let options = parse(args)?;
    if options.help {
        help();
        return Ok(());
    }
    if !options.copies.is_empty() {
        if options.clipboard || !options.inputs.is_empty() || options.playlist {
            return Err(ToolError::usage(
                TOOL,
                "--copy cannot be combined with links, --clipboard, or --playlist",
            ));
        }
        let output = options.output.unwrap_or_else(|| PathBuf::from("."));
        let result = save_snapshot(
            clipboard::Snapshot::Files {
                paths: options.copies,
                cut: false,
            },
            &output,
            &options.settings,
            options.dry_run,
        )
        .map_err(|e| ToolError::new(TOOL, e))?;
        println!("{TOOL}: {result}");
        return Ok(());
    }
    let mut urls = links(&options.inputs.join("\n"));
    if options.inputs.iter().any(|input| links(input).is_empty()) {
        return Err(ToolError::usage(
            TOOL,
            "every input must be an http(s) link or a Git repository link",
        ));
    }
    if options.clipboard {
        let snapshot = clipboard::read().map_err(|message| ToolError::new(TOOL, message))?;
        let text = match snapshot {
            clipboard::Snapshot::Text(text) if !links(&text).is_empty() => text,
            snapshot => {
                if !urls.is_empty() {
                    return Err(ToolError::usage(
                        TOOL,
                        "do not combine URLs with non-link clipboard content",
                    ));
                }
                let output = options.output.clone().unwrap_or_else(|| PathBuf::from("."));
                let result = save_snapshot(snapshot, &output, &options.settings, options.dry_run)
                    .map_err(|e| ToolError::new(TOOL, e))?;
                println!("{TOOL}: {result}");
                return Ok(());
            }
        };
        let copied = links(&text);
        if copied.is_empty() {
            return Err(ToolError::new(
                TOOL,
                "the clipboard holds no supported link; copy a link and try again",
            ));
        }
        for link in copied {
            if !urls.contains(&link) {
                urls.push(link);
            }
        }
    } else if urls.is_empty() && !common::stdin_is_terminal() {
        let text = common::read_stdin()?;
        urls = links(&text);
        if urls.is_empty() && options.settings.text {
            let output = options.output.as_deref().unwrap_or(Path::new("."));
            let result = save_snapshot(
                clipboard::Snapshot::Text(text),
                output,
                &options.settings,
                options.dry_run,
            )
            .map_err(|e| ToolError::new(TOOL, e))?;
            println!("{TOOL}: {result}");
            return Ok(());
        }
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
            if let Some(repository) = git::repository(url) {
                println!(
                    "  {} -> {} (git clone; full repository, no overwrite)",
                    repository.remote,
                    common::display_path(&git::destination(&directory, &repository.name))
                );
            } else {
                println!(
                    "  {url} -> {} (the file itself, or yt-dlp for a page)",
                    common::display_path(&directory)
                );
            }
        }
        return Ok(());
    }
    fs::create_dir_all(&directory).map_err(|error| {
        ToolError::new(
            TOOL,
            format!("cannot create {}: {error}", directory.display()),
        )
    })?;
    if urls.iter().any(|url| git::repository(url).is_none()) {
        download::refresh(TOOL);
    }
    let agent = agent();
    let mut downloader = Downloader { resolved: None };
    let mut saved = 0;
    let mut cloned = 0;
    let mut failed = 0;
    for url in &urls {
        if common::interrupted() {
            break;
        }
        println!("{TOOL}: {url}");
        match paste(&agent, &mut downloader, url, options.playlist, &directory) {
            Ok(files) => {
                for file in files {
                    match file {
                        Pasted::File(path, size) => {
                            saved += 1;
                            println!(
                                "  saved   {}  {}",
                                common::display_path(&path),
                                common::format_bytes(size)
                            );
                        }
                        Pasted::Repository(path) => {
                            cloned += 1;
                            println!("  cloned  {}", common::display_path(&path));
                        }
                    }
                }
            }
            Err(reason) => {
                failed += 1;
                eprintln!("  failed  {url}: {reason}");
            }
        }
    }
    let mut summary = Vec::new();
    if saved > 0 || cloned == 0 {
        summary.push(format!("{saved} saved"));
    }
    if cloned > 0 {
        summary.push(format!("{cloned} cloned"));
    }
    if failed > 0 {
        summary.push(format!("{failed} failed"));
    }
    println!("{TOOL}: {}", summary.join(", "));
    if failed > 0 {
        Err(ToolError::new(TOOL, "some links could not be pasted"))
    } else if common::interrupted() {
        Err(ToolError::cancelled(TOOL))
    } else {
        Ok(())
    }
}

fn save_snapshot(
    snapshot: clipboard::Snapshot,
    directory: &Path,
    settings: &Settings,
    dry_run: bool,
) -> Result<String, String> {
    if common::interrupted() {
        return Err("cancelled".into());
    }
    match snapshot {
        clipboard::Snapshot::Files { paths, cut } => {
            if cut {
                return Err("cut/move belongs to your file manager; copy the files first".into());
            }
            let plan = copy::plan(&paths, directory, settings)?;
            if dry_run {
                for (source, target, _) in &plan.entries {
                    println!("  {} -> {}", source.display(), target.display());
                }
                return Ok(format!(
                    "dry run: {} entries; {} links or selected roots skipped; ignore rules applied",
                    plan.entries.len(),
                    plan.skipped
                ));
            }
            let count = copy::execute(&plan)?;
            Ok(format!(
                "copied {count} files; {} links or selected roots skipped; ignore rules applied",
                plan.skipped
            ))
        }
        clipboard::Snapshot::Image(bytes, format) => {
            let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(32768);
            limits.max_image_height = Some(32768);
            limits.max_alloc = Some(256 * 1024 * 1024);
            reader.limits(limits);
            let image = reader
                .decode()
                .map_err(|e| format!("cannot decode clipboard image: {e}"))?;
            let name = format!(
                "Image {}.png",
                chrono::Local::now().format("%Y-%m-%d %H-%M-%S")
            );
            if dry_run {
                return Ok(format!(
                    "dry run: {}x{} PNG -> {}",
                    image.width(),
                    image.height(),
                    directory.join(name).display()
                ));
            }
            fs::create_dir_all(directory).map_err(|e| e.to_string())?;
            let mut temp = tempfile::Builder::new()
                .prefix(".justpaste-")
                .tempfile_in(directory)
                .map_err(|e| e.to_string())?;
            if format == image::ImageFormat::Png {
                // Validate above, then preserve the original PNG bytes, color
                // metadata and transparency instead of converting a browser copy.
                temp.write_all(&bytes).map_err(|e| e.to_string())?;
            } else {
                image
                    .write_to(temp.as_file_mut(), image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
            }
            let path = free_path(directory, &name);
            if common::interrupted() {
                return Err("cancelled".into());
            }
            temp.persist_noclobber(&path).map_err(|e| e.to_string())?;
            Ok(format!(
                "saved {} ({}x{})",
                path.display(),
                image.width(),
                image.height()
            ))
        }
        clipboard::Snapshot::Text(text) if settings.text => {
            let name = format!(
                "Text {}.txt",
                chrono::Local::now().format("%Y-%m-%d %H-%M-%S")
            );
            if dry_run {
                return Ok(format!(
                    "dry run: UTF-8 text -> {}",
                    directory.join(name).display()
                ));
            }
            fs::create_dir_all(directory).map_err(|e| e.to_string())?;
            let mut temp = tempfile::Builder::new()
                .prefix(".justpaste-")
                .tempfile_in(directory)
                .map_err(|e| e.to_string())?;
            temp.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            let path = free_path(directory, &name);
            if common::interrupted() {
                return Err("cancelled".into());
            }
            temp.persist_noclobber(&path).map_err(|e| e.to_string())?;
            Ok(format!("saved {}", path.display()))
        }
        _ => Err(
            "clipboard has no supported image, files, or links; use --text for plain text".into(),
        ),
    }
}

#[cfg(windows)]
fn smart_paste(
    snapshot: clipboard::Snapshot,
    directory: &Path,
    settings: &Settings,
) -> Result<String, String> {
    if let clipboard::Snapshot::Text(text) = &snapshot {
        let urls = links(text);
        if !urls.is_empty() {
            let mut downloader = Downloader { resolved: None };
            let agent = agent();
            let mut saved = 0;
            let mut cloned = 0;
            for url in urls {
                if common::interrupted() {
                    return Err(format!(
                        "cancelled; {saved} files saved, {cloned} repositories cloned"
                    ));
                }
                for result in paste(&agent, &mut downloader, &url, false, directory)? {
                    match result {
                        Pasted::File(..) => saved += 1,
                        Pasted::Repository(_) => cloned += 1,
                    }
                }
            }
            return Ok(if cloned > 0 {
                if saved > 0 {
                    format!("saved {saved} files; cloned {cloned} repositories")
                } else {
                    format!("cloned {cloned} Git repositories")
                }
            } else {
                format!("saved {saved} downloaded files")
            });
        }
    }
    save_snapshot(snapshot, directory, settings, false)
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
        assert_eq!(
            links(
                "Clone <git@github.com:owner/repo.git>, or `ssh://git@forge.test/team/repo` and git://forge.test/team/repo."
            ),
            [
                "git@github.com:owner/repo.git",
                "ssh://git@forge.test/team/repo",
                "git://forge.test/team/repo"
            ]
        );
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
            "report final.pdf"
        );
        // A name no folder should have to hold: traversal, emoji, hashtags,
        // and far more characters than a path can spare.
        assert_eq!(
            file_name(
                "https://example.test/get",
                Some("attachment; filename=\"../../so good 😭😭 #fyp.MP4\""),
                "video/mp4"
            ),
            "so good.MP4"
        );
        let long = file_name(
            &format!("https://cdn.example.test/v/{}.jpg:large", "a1B2".repeat(90)),
            None,
            "image/jpeg",
        );
        assert!(
            long.ends_with(".jpg") && long.chars().count() <= 84,
            "{long}"
        );
        assert_eq!(
            file_name("https://example.test/%F0%9F%98%AD.png", None, "image/png"),
            "example.test.png"
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
        assert_eq!(
            file_name("https://example.test/con.txt", None, "text/plain"),
            "_con.txt"
        );
        assert_eq!(
            file_name("https://example.test/trailing.%20", None, ""),
            "trailing"
        );
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

    #[test]
    fn image_snapshot_preserves_rgba_dimensions_and_dry_run_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let image = image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([
                x as u8 * 60,
                y as u8 * 70,
                120,
                if x == 0 { 0 } else { 128 },
            ])
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let snapshot =
            || clipboard::Snapshot::Image(encoded.get_ref().clone(), image::ImageFormat::Png);
        save_snapshot(snapshot(), directory.path(), &Settings::default(), true).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        for _ in 0..2 {
            save_snapshot(snapshot(), directory.path(), &Settings::default(), false).unwrap();
        }
        let paths: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(paths.len(), 2);
        for path in paths {
            assert_eq!(path.extension().unwrap(), "png");
            assert_eq!(fs::read(&path).unwrap(), *encoded.get_ref());
            assert_eq!(image::open(path).unwrap().to_rgba8(), image);
        }
    }

    #[test]
    fn invalid_clipboard_image_leaves_no_output() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = clipboard::Snapshot::Image(b"invalid PNG".to_vec(), image::ImageFormat::Png);
        assert!(save_snapshot(snapshot, directory.path(), &Settings::default(), false).is_err());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn cut_payload_and_disabled_text_leave_output_untouched() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            save_snapshot(
                clipboard::Snapshot::Files {
                    paths: vec![],
                    cut: true
                },
                directory.path(),
                &Settings::default(),
                false
            )
            .is_err()
        );
        assert!(
            save_snapshot(
                clipboard::Snapshot::Text("hello".into()),
                directory.path(),
                &Settings::default(),
                false
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        let settings = Settings {
            text: true,
            ..Default::default()
        };
        save_snapshot(
            clipboard::Snapshot::Text("hello 🌍".into()),
            directory.path(),
            &settings,
            false,
        )
        .unwrap();
        let file = fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read_to_string(file).unwrap(), "hello 🌍");
    }
}
