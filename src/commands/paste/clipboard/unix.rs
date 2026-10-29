//! Read macOS pasteboard and Linux Wayland/X11 formats without changing them.
#![cfg_attr(windows, allow(dead_code))]
use super::Snapshot;
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_TYPES: usize = 1024 * 1024;

fn command(program: &str, arguments: &[&str], limit: usize) -> Result<Option<Vec<u8>>, String> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::commands::download::hide_console(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read the clipboard with {program}: {error}")),
    };
    let output = child.stdout.take().ok_or("missing clipboard output pipe")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > limit {
            return Err("clipboard content exceeds the size limit".to_owned());
        }
        Ok(bytes)
    });
    let status =
        crate::commands::download::wait_child(&mut child, Some(Duration::from_secs(10)), true);
    let output = reader
        .join()
        .map_err(|_| "cannot read clipboard output")??;
    let status = status.map_err(|e| format!("clipboard reader {program}: {e}"))?;
    Ok(status.success().then_some(output))
}

#[cfg(any(not(target_os = "macos"), test))]
fn file_uri(uri: &str) -> Result<PathBuf, String> {
    let rest = uri
        .strip_prefix("file://")
        .ok_or("copied files must use local file:// URLs")?;
    let path = if rest.starts_with('/') {
        rest.to_owned()
    } else if let Some(local) = rest.strip_prefix("localhost/") {
        format!("/{local}")
    } else {
        return Err(
            "remote file clipboard URLs are not supported; copy a local file instead".into(),
        );
    };
    let mut bytes = Vec::new();
    let mut input = path.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        let byte = if byte == b'%' {
            let high = input.next().and_then(|b| (b as char).to_digit(16));
            let low = input.next().and_then(|b| (b as char).to_digit(16));
            match (high, low) {
                (Some(high), Some(low)) => (high * 16 + low) as u8,
                _ => return Err("invalid percent encoding in copied file URL".into()),
            }
        } else {
            byte
        };
        if byte == 0 {
            return Err("invalid NUL in copied file URL".into());
        }
        bytes.push(byte);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(std::ffi::OsString::from_vec(bytes).into())
    }
    #[cfg(not(unix))]
    Ok(String::from_utf8(bytes).map_err(|e| e.to_string())?.into())
}

#[cfg(any(not(target_os = "macos"), test))]
fn uri_snapshot(bytes: &[u8], gnome: bool, kde_cut: bool) -> Result<Snapshot, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "copied file URLs must be UTF-8")?;
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let cut = if gnome {
        match lines.next() {
            Some("copy") => false,
            Some("cut") => true,
            _ => return Err("unsupported copied-file action; copy the files again".into()),
        }
    } else {
        kde_cut
    };
    let urls: Vec<_> = lines.filter(|line| !line.starts_with('#')).collect();
    if urls.len() > 100_000 {
        return Err("too many copied files".into());
    }
    if !gnome && !urls.iter().any(|url| url.starts_with("file:")) {
        return Ok(Snapshot::Text(urls.join("\n")));
    }
    let paths = urls
        .into_iter()
        .map(file_uri)
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Err("the clipboard holds no copied files".into());
    }
    Ok(Snapshot::Files { paths, cut })
}

#[cfg(not(target_os = "macos"))]
fn linux_snapshot(program: &str, types: &[u8]) -> Result<Snapshot, String> {
    let types = String::from_utf8_lossy(types);
    let has = |mime: &str| types.split_whitespace().any(|value| value == mime);
    let read = |mime: &str| -> Result<Vec<u8>, String> {
        let args = if program == "wl-paste" {
            vec!["--no-newline", "--type", mime]
        } else {
            vec!["-selection", "clipboard", "-o", "-t", mime]
        };
        command(program, &args, MAX_BYTES)?
            .ok_or_else(|| "clipboard changed before paste; copy it again".into())
    };
    if has("x-special/gnome-copied-files") {
        return uri_snapshot(&read("x-special/gnome-copied-files")?, true, false);
    }
    if has("text/uri-list") {
        let cut = has("application/x-kde-cutselection")
            && read("application/x-kde-cutselection")?.starts_with(b"1");
        return uri_snapshot(&read("text/uri-list")?, false, cut);
    }
    for (mime, format) in [
        ("image/png", image::ImageFormat::Png),
        ("image/bmp", image::ImageFormat::Bmp),
        ("image/jpeg", image::ImageFormat::Jpeg),
        ("image/tiff", image::ImageFormat::Tiff),
        ("image/webp", image::ImageFormat::WebP),
    ] {
        if has(mime) {
            return Ok(Snapshot::Image(read(mime)?, format));
        }
    }
    for mime in [
        "text/plain;charset=utf-8",
        "UTF8_STRING",
        "text/plain",
        "TEXT",
        "STRING",
    ] {
        if has(mime) {
            let text = String::from_utf8_lossy(&read(mime)?).into_owned();
            if has("text/html") {
                let html = String::from_utf8_lossy(&read("text/html")?)
                    .trim()
                    .to_owned();
                if !html.is_empty() {
                    return Ok(Snapshot::Html { html, text });
                }
            }
            return Ok(Snapshot::Text(text));
        }
    }
    Ok(Snapshot::Unsupported)
}

#[cfg(any(target_os = "macos", test))]
const MACOS_READ: &str = r#"
ObjC.import('AppKit');
var board = $.NSPasteboard.generalPasteboard;
var count = board.changeCount;
var result = {kind: 'unsupported'};
var urls = board.readObjectsForClassesOptions($.NSArray.arrayWithObject($.NSURL), $.NSDictionary.dictionary);
var paths = [];
if (urls && urls.count > 100000) throw new Error('Too many copied files');
if (urls) for (var i = 0; i < urls.count; i++) {
    var url = urls.objectAtIndex(i);
    if (url.isFileURL) paths.push(ObjC.unwrap(url.path));
}
if (paths.length) result = {kind: 'files', paths: paths};
if (result.kind === 'unsupported') {
    var formats = [['public.png', 'png'], ['public.tiff', 'tiff'], ['public.jpeg', 'jpeg']];
    for (var i = 0; i < formats.length; i++) {
        var data = board.dataForType($(formats[i][0]));
        if (data && data.length) {
            if (data.length > 268435456) throw new Error('Clipboard image exceeds the size limit');
            result = {kind: 'image', format: formats[i][1], data: ObjC.unwrap(data.base64EncodedStringWithOptions(0))};
            break;
        }
    }
}
if (result.kind === 'unsupported') {
    var text = board.stringForType($('public.utf8-plain-text'));
    if (!text || !text.length) text = board.stringForType($('public.url'));
    var html = board.stringForType($('public.html'));
    if (text && html && html.length) result = {kind: 'html', text: ObjC.unwrap(text), html: ObjC.unwrap(html)};
    else if (text) result = {kind: 'text', text: ObjC.unwrap(text)};
}
if (board.changeCount !== count) throw new Error('Clipboard changed before paste; copy it again');
JSON.stringify(result);
"#;

#[cfg(any(target_os = "macos", test))]
fn macos_snapshot(bytes: &[u8]) -> Result<Snapshot, String> {
    use base64::Engine;
    #[derive(serde::Deserialize)]
    #[serde(tag = "kind", rename_all = "lowercase")]
    enum Payload {
        Files { paths: Vec<PathBuf> },
        Image { format: String, data: String },
        Text { text: String },
        Html { text: String, html: String },
        Unsupported,
    }
    match serde_json::from_slice(bytes).map_err(|e| format!("cannot read macOS clipboard: {e}"))? {
        Payload::Files { paths } if paths.len() <= 100_000 => {
            Ok(Snapshot::Files { paths, cut: false })
        }
        Payload::Files { .. } => Err("too many copied files".into()),
        Payload::Image { format, data } => {
            let format = match format.as_str() {
                "png" => image::ImageFormat::Png,
                "tiff" => image::ImageFormat::Tiff,
                "jpeg" => image::ImageFormat::Jpeg,
                _ => return Err("unsupported clipboard image format".into()),
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| e.to_string())?;
            if bytes.len() > MAX_BYTES {
                return Err("clipboard image exceeds the size limit".into());
            }
            Ok(Snapshot::Image(bytes, format))
        }
        Payload::Text { text } => Ok(Snapshot::Text(text)),
        Payload::Html { text, html } => Ok(Snapshot::Html { html, text }),
        Payload::Unsupported => Ok(Snapshot::Unsupported),
    }
}

pub fn read() -> Result<Snapshot, String> {
    #[cfg(target_os = "macos")]
    {
        let bytes = command("/usr/bin/osascript", &["-l", "JavaScript", "-e", MACOS_READ], MAX_BYTES * 4 / 3 + MAX_TYPES)?
            .ok_or("cannot access the macOS clipboard; run in a logged-in desktop session, or pass a URL, --copy PATH, or pipe text instead")?;
        macos_snapshot(&bytes)
    }
    #[cfg(not(target_os = "macos"))]
    {
        for (program, arguments) in [
            ("wl-paste", vec!["--list-types"]),
            (
                "xclip",
                vec!["-selection", "clipboard", "-o", "-t", "TARGETS"],
            ),
        ] {
            if let Some(types) = command(program, &arguments, MAX_TYPES)? {
                return linux_snapshot(program, &types);
            }
        }
        if let Some(text) = command("xsel", &["--clipboard", "--output"], MAX_BYTES)? {
            return Ok(Snapshot::Text(String::from_utf8_lossy(&text).into_owned()));
        }
        Err("cannot access the Linux clipboard. Wayland needs wl-paste (wl-clipboard); X11 needs xclip (images/files/text) or xsel (text). Use a desktop session, or pass a URL, --copy PATH, or pipe text instead. Nothing was installed.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copied_file_uris_preserve_spaces_unicode_and_cut_intent() {
        let Snapshot::Files { paths, cut } = uri_snapshot(
            b"cut\nfile:///tmp/a%20b%23c.txt\nfile://localhost/tmp/caf%C3%A9\n",
            true,
            false,
        )
        .unwrap() else {
            panic!()
        };
        assert!(cut);
        assert_eq!(
            paths,
            [PathBuf::from("/tmp/a b#c.txt"), PathBuf::from("/tmp/café")]
        );
        for uri in [
            "file://remote/tmp/file",
            "file:///tmp/%00x",
            "file:///tmp/%zz",
            "https://example.com/a",
        ] {
            assert!(file_uri(uri).is_err(), "{uri}");
        }
        assert!(uri_snapshot(b"move\nfile:///tmp/file", true, false).is_err());
        assert!(matches!(
            uri_snapshot(b"# URLs\r\nhttps://github.com/owner/repo\r\n", false, false).unwrap(),
            Snapshot::Text(_)
        ));
        assert!(matches!(
            uri_snapshot(b"file:///tmp/a\n", false, true).unwrap(),
            Snapshot::Files { cut: true, .. }
        ));
    }
    #[test]
    fn macos_payload_handles_images_files_and_empty_clipboard() {
        assert!(
            matches!(macos_snapshot(br#"{"kind":"image","format":"png","data":"UE5H"}"#).unwrap(), Snapshot::Image(data, image::ImageFormat::Png) if data == b"PNG")
        );
        assert!(matches!(
            macos_snapshot(br#"{"kind":"files","paths":["/tmp/a b"]}"#).unwrap(),
            Snapshot::Files { cut: false, .. }
        ));
        assert!(matches!(
            macos_snapshot(br#"{"kind":"unsupported"}"#).unwrap(),
            Snapshot::Unsupported
        ));
        assert!(macos_snapshot(br#"{"kind":"image","format":"png","data":"invalid!"}"#).is_err());
        assert!(MACOS_READ.contains("changeCount"));
    }
}
