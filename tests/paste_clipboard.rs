#![cfg(unix)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
#[cfg(target_os = "linux")]
use std::{io::Write, process::Stdio};

fn binary() -> PathBuf {
    std::env::var_os("JUSTTOOLS_PASTE_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_just").into())
}

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(2, 2, |x, y| {
        image::Rgba([
            x as u8 * 100,
            y as u8 * 100,
            40,
            if x == 0 { 0 } else { 128 },
        ])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn paste(output: &Path, extra: &[&str]) -> Command {
    let mut command = Command::new(binary());
    command
        .args(["paste", "--clipboard", "--output"])
        .arg(output)
        .args(extra);
    command
}

fn assert_success(result: &std::process::Output) {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[cfg(target_os = "linux")]
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.') {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(target_os = "linux")]
fn fake_wayland(root: &Path, mime: &str, payload: &[u8]) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("types"), mime).unwrap();
    fs::write(bin.join("payload"), payload).unwrap();
    let quoted = format!("'{}'", bin.display().to_string().replace('\'', "'\\''"));
    fs::write(bin.join("wl-paste"), format!("#!/bin/sh\ncase \"$*\" in\n--list-types) /bin/cat {quoted}/types ;;\n'--no-newline --type {mime}') /bin/cat {quoted}/payload ;;\n*) exit 1 ;;\nesac\n")).unwrap();
    fs::set_permissions(bin.join("wl-paste"), fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[cfg(target_os = "linux")]
#[test]
fn wayland_formats_preserve_png_and_files_and_refuse_cut_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    fs::create_dir(&output).unwrap();
    let bytes = png();
    let bin = fake_wayland(temp.path(), "image/png", &bytes);
    let preview = paste(&output, &["--dry-run"])
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert_success(&preview);
    assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    assert_success(&paste(&output, &[]).env("PATH", &bin).output().unwrap());
    let image = fs::read_dir(&output)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read(image).unwrap(), bytes);
    let source = temp.path().join("a space # café.txt");
    fs::write(&source, "keep source").unwrap();
    let uri = file_uri(&source);
    let bin = fake_wayland(
        temp.path(),
        "x-special/gnome-copied-files",
        format!("copy\n{uri}\n").as_bytes(),
    );
    assert_success(&paste(&output, &[]).env("PATH", &bin).output().unwrap());
    assert_eq!(
        fs::read_to_string(output.join(source.file_name().unwrap())).unwrap(),
        "keep source"
    );
    let bin = fake_wayland(
        temp.path(),
        "x-special/gnome-copied-files",
        format!("cut\n{uri}\n").as_bytes(),
    );
    let cut = paste(&output, &[]).env("PATH", &bin).output().unwrap();
    assert!(!cut.status.success());
    assert!(String::from_utf8_lossy(&cut.stderr).contains("cut/move"));
    assert_eq!(fs::read_to_string(&source).unwrap(), "keep source");
    assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
}

#[cfg(target_os = "linux")]
#[test]
fn missing_linux_clipboard_helpers_explain_cli_alternatives() {
    let temp = tempfile::tempdir().unwrap();
    let result = paste(temp.path(), &[])
        .env("PATH", temp.path())
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("wl-paste") && error.contains("xclip") && error.contains("pipe text"));
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "writes an isolated X11 clipboard; run under xvfb-run, never a user's desktop"]
fn x11_native_clipboard_handles_images_files_and_text() {
    assert!(std::env::var("JUSTTOOLS_TEST_X11_ISOLATED").is_ok_and(|v| v == "1"));
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    fs::create_dir(&output).unwrap();
    let set = |mime: &str, data: &[u8]| {
        let mut child = Command::new("xclip")
            .args(["-selection", "clipboard", "-t", mime, "-i"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(data).unwrap();
        assert!(child.wait().unwrap().success());
    };
    let bytes = png();
    set("image/png", &bytes);
    assert_success(
        &paste(&output, &[])
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap(),
    );
    let saved = fs::read_dir(&output)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read(saved).unwrap(), bytes);
    let source = temp.path().join("a space.txt");
    fs::write(&source, "source kept").unwrap();
    set("text/uri-list", file_uri(&source).as_bytes());
    assert_success(
        &paste(&output, &[])
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap(),
    );
    assert!(source.is_file() && output.join("a space.txt").is_file());
    set("UTF8_STRING", b"clipboard text\n");
    assert_success(
        &paste(&output, &["--text"])
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap(),
    );
    let text = fs::read_dir(&output)
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.file_name().to_string_lossy().starts_with("Text "))
        .unwrap();
    assert_eq!(fs::read(text.path()).unwrap(), b"clipboard text\n");
}

#[cfg(target_os = "macos")]
fn macos(script: &str) -> String {
    let result = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()
        .unwrap();
    assert_success(&result);
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

#[cfg(target_os = "macos")]
struct RestoreMacClipboard(String);
#[cfg(target_os = "macos")]
impl Drop for RestoreMacClipboard {
    fn drop(&mut self) {
        let script = format!(
            r#"ObjC.import('AppKit'); var board = $.NSPasteboard.generalPasteboard; var saved = {}; var items = []; for (var i = 0; i < saved.length; i++) {{ var item = $.NSPasteboardItem.alloc.init; for (var j = 0; j < saved[i].length; j++) {{ var value = saved[i][j]; var data = $.NSData.alloc.initWithBase64EncodedStringOptions($(value.data), 0); item.setDataForType(data, $(value.type)); }} items.push(item); }} board.clearContents; if (items.length) board.writeObjects($(items));"#,
            self.0
        );
        let _ = Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", &script])
            .output();
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "uses the macOS pasteboard briefly and restores its existing formats"]
fn macos_native_clipboard_handles_images_files_and_text() {
    use base64::Engine;
    let original = macos(
        r#"ObjC.import('AppKit'); var board = $.NSPasteboard.generalPasteboard; var items = board.pasteboardItems; var saved = []; if (items) for (var i = 0; i < items.count; i++) { var item = items.objectAtIndex(i); var values = []; for (var j = 0; j < item.types.count; j++) { var type = item.types.objectAtIndex(j); var data = item.dataForType(type); if (data) values.push({type: ObjC.unwrap(type), data: ObjC.unwrap(data.base64EncodedStringWithOptions(0))}); } saved.push(values); } JSON.stringify(saved);"#,
    );
    let _restore = RestoreMacClipboard(original);
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    fs::create_dir(&output).unwrap();
    let bytes = png();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    macos(&format!(
        r#"ObjC.import('AppKit'); var board = $.NSPasteboard.generalPasteboard; board.clearContents; board.setDataForType($.NSData.alloc.initWithBase64EncodedStringOptions($('{encoded}'), 0), $('public.png'));"#
    ));
    let before = macos("ObjC.import('AppKit'); $.NSPasteboard.generalPasteboard.changeCount;");
    assert_success(&paste(&output, &[]).output().unwrap());
    assert_eq!(
        macos("ObjC.import('AppKit'); $.NSPasteboard.generalPasteboard.changeCount;"),
        before
    );
    let saved = fs::read_dir(&output)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read(saved).unwrap(), bytes);
    let source = temp.path().join("a space.txt");
    fs::write(&source, "source kept").unwrap();
    let path = serde_json::to_string(&source.to_string_lossy()).unwrap();
    macos(&format!(
        "ObjC.import('AppKit'); var board = $.NSPasteboard.generalPasteboard; board.clearContents; board.writeObjects($.NSArray.arrayWithObject($.NSURL.fileURLWithPath($({path}))));"
    ));
    assert_success(&paste(&output, &[]).output().unwrap());
    assert!(source.is_file() && output.join("a space.txt").is_file());
    macos(
        "ObjC.import('AppKit'); var board = $.NSPasteboard.generalPasteboard; board.clearContents; board.setStringForType($('clipboard text'), $('public.utf8-plain-text'));",
    );
    assert_success(&paste(&output, &["--text"]).output().unwrap());
    let text = fs::read_dir(&output)
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.file_name().to_string_lossy().starts_with("Text "))
        .unwrap();
    assert_eq!(fs::read(text.path()).unwrap(), b"clipboard text");
}
