use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const KEY: &str = "fixture-secret-never-print";

struct Host {
    url: String,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    thread: Option<thread::JoinHandle<()>>,
    png: Vec<u8>,
}
impl Host {
    fn new(fail: bool, multiple: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let origin = url.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let image = image::RgbaImage::from_fn(2, 2, |x, _| {
            image::Rgba([50, 90, 150, if x == 0 { 0 } else { 128 }])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();
        let photo = png.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let header_end = loop {
                    let count = stream.read(&mut buffer).unwrap_or(0);
                    if count == 0 {
                        break None;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break Some(end + 4);
                    }
                    if bytes.len() > 32 * 1024 {
                        break None;
                    }
                };
                let Some(header_end) = header_end else {
                    continue;
                };
                let length = String::from_utf8_lossy(&bytes[..header_end])
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let count = stream.read(&mut buffer).unwrap_or(0);
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let request = String::from_utf8_lossy(&bytes).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                captured.lock().unwrap().push(request);
                let (status, kind, body) = if path == "/avatar" {
                    ("200 OK", "image/png", photo.clone())
                } else if path == "/article" {
                    ("200 OK", "text/html", format!("<title>Notes &amp; links</title><meta property='og:image' content='{origin}/avatar'><h1>Article</h1>").into_bytes())
                } else if path == "/profile" {
                    (
                        "200 OK",
                        "text/html",
                        format!("<meta property='og:image' content='{origin}/avatar'>")
                            .into_bytes(),
                    )
                } else if path == "/media" {
                    (
                        "200 OK",
                        "text/html",
                        format!(
                            "<video></video><meta property='og:image' content='{origin}/avatar'>"
                        )
                        .into_bytes(),
                    )
                } else if path.starts_with("/accounts?") {
                    (
                        "200 OK",
                        "application/json",
                        if multiple {
                            format!(r#"{{"success":true,"result":[{{"id":"{ACCOUNT}"}},{{"id":"fedcba9876543210fedcba9876543210"}}]}}"#).into_bytes()
                        } else {
                            br#"{"success":true,"result":[]}"#.to_vec()
                        },
                    )
                } else if path.starts_with("/zones?") {
                    (
                        "200 OK",
                        "application/json",
                        format!(
                            r#"{{"success":true,"result":[{{"account":{{"id":"{ACCOUNT}"}}}}]}}"#
                        )
                        .into_bytes(),
                    )
                } else if path.ends_with("/browser-run/markdown") {
                    if fail {
                        ("401 Unauthorized", "application/json", format!(r#"{{"success":false,"errors":[{{"code":10000,"message":"{KEY}"}}]}}"#).into_bytes())
                    } else {
                        ("200 OK", "application/json", br##"{"success":true,"result":"# Rendered article\n\n[Link](https://example.test)"}"##.to_vec())
                    }
                } else {
                    ("404 Not Found", "text/plain", b"missing".to_vec())
                };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).ok();
                stream.write_all(&body).ok();
            }
        });
        Self {
            url,
            stop,
            requests,
            thread: Some(thread),
            png,
        }
    }
    fn command(&self, output: &Path) -> Command {
        let binary = std::env::var_os("JUSTTOOLS_PASTE_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_just").into());
        let mut command = Command::new(binary);
        command
            .args(["paste", "--output"])
            .arg(output)
            .env("CLOUDFLARE_API_TOKEN", "")
            .env("CLOUDFLARE_API_KEY", KEY)
            .env("CLOUDFLARE_EMAIL", "")
            .env("CLOUDFLARE_ACCOUNT_ID", "")
            .env("CF_ACCOUNT_ID", "")
            .env("JUSTTOOLS_CLOUDFLARE_TEST_BASE", &self.url)
            .env("YTDLP_BIN", output.join("missing-ytdlp"))
            .env("FFMPEG_BIN", output.join("missing-ffmpeg"))
            .env("JUSTTOOLS_DEFAULTS", output.with_extension("defaults.toml"));
        command
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn success(result: &std::process::Output) {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn cloudflare_pages_become_markdown_with_account_discovery_and_numbered_names() {
    let host = Host::new(false, false);
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    for _ in 0..2 {
        success(
            &host
                .command(&output)
                .arg(format!("{}/article", host.url))
                .output()
                .unwrap(),
        );
    }
    for name in ["Notes & links.md", "Notes & links (2).md"] {
        assert_eq!(
            fs::read_to_string(output.join(name)).unwrap(),
            "# Rendered article\n\n[Link](https://example.test)\n"
        );
    }
    let requests = host.requests.lock().unwrap();
    let markdown: Vec<_> = requests
        .iter()
        .filter(|r| r.contains("/browser-run/markdown"))
        .collect();
    assert_eq!(markdown.len(), 2);
    assert!(
        markdown[0]
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {KEY}"))
    );
    assert!(markdown[0].contains("networkidle2") && markdown[0].contains("/article"));
    assert!(
        requests
            .iter()
            .filter(|r| r.starts_with("GET /article"))
            .all(|r| !r.contains(KEY))
    );
    assert!(!output.with_extension("defaults.toml").exists());
}

#[test]
fn markdown_errors_redact_credentials_and_account_ambiguity_prevents_conversion() {
    for (fail, multiple) in [(true, false), (false, true)] {
        let host = Host::new(fail, multiple);
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let result = host
            .command(&output)
            .args(["--markdown", "https://example.test/page"])
            .output()
            .unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(!error.contains(KEY));
        assert!(error.contains(if fail {
            "HTTP 401"
        } else {
            "multiple accounts"
        }));
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
        if multiple {
            assert!(
                !host
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| r.contains("/browser-run/markdown"))
            );
        }
    }
}

#[test]
fn public_profile_photos_keep_png_alpha_sources_and_number_collisions() {
    let host = Host::new(false, false);
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    for _ in 0..2 {
        success(
            &host
                .command(&output)
                .env(
                    "JUSTTOOLS_PROFILE_TEST_PAGE",
                    format!("{}/profile", host.url),
                )
                .arg("https://facebook.com/fixture")
                .output()
                .unwrap(),
        );
    }
    for name in [
        "facebook-fixture-profile.png",
        "facebook-fixture-profile (2).png",
    ] {
        let file = output.join(name);
        assert_eq!(fs::read(&file).unwrap(), host.png);
        let image = image::open(file).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert_eq!(image.get_pixel(1, 0)[3], 128);
    }
    assert!(
        !host
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("browser-run"))
    );
}

#[test]
fn media_pages_keep_the_downloader_and_declared_image_fallback_when_cloudflare_is_enabled() {
    let host = Host::new(false, false);
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    success(
        &host
            .command(&output)
            .arg(format!("{}/media", host.url))
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(output.join("avatar.png")).unwrap(), host.png);
    assert!(
        !host
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("browser-run"))
    );
}

#[test]
fn web_modes_preview_without_network_and_cannot_be_combined() {
    let host = Host::new(false, false);
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    for args in [
        ["--markdown", "https://example.test/page"],
        ["--profile-photo", "https://x.com/user"],
    ] {
        success(
            &host
                .command(&output)
                .arg("--dry-run")
                .args(args)
                .output()
                .unwrap(),
        );
    }
    assert!(!output.exists());
    assert!(host.requests.lock().unwrap().is_empty());
    let result = host
        .command(&output)
        .args(["--markdown", "--profile-photo", "https://x.com/user"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
}
