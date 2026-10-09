use std::{fs, path::Path, process::Command};

fn binary() -> std::path::PathBuf {
    std::env::var_os("JUSTTOOLS_PASTE_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_just").into())
}

struct GitFixture {
    temp: tempfile::TempDir,
    source: std::path::PathBuf,
    output: std::path::PathBuf,
    config: std::path::PathBuf,
    remote: &'static str,
}

impl GitFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source repo");
        let output = temp.path().join("pasted directory");
        let config = temp.path().join("gitconfig");
        fs::create_dir(&source).unwrap();
        fs::write(&config, "").unwrap();
        let fixture = Self {
            temp,
            source,
            output,
            config,
            remote: "https://github.com/justpaste-fixture/repo.name",
        };
        fixture.git(
            &fixture.source,
            &["init", "--quiet", "--initial-branch=main"],
        );
        fs::write(fixture.source.join("README.md"), "first revision\n").unwrap();
        fixture.git(&fixture.source, &["add", "README.md"]);
        fixture.git(
            &fixture.source,
            &["commit", "--quiet", "-m", "First revision"],
        );
        fixture.git(&fixture.source, &["branch", "other-branch"]);
        fixture.git(&fixture.source, &["tag", "fixture-tag"]);
        fs::write(fixture.source.join("README.md"), "second revision\n").unwrap();
        fixture.git(
            &fixture.source,
            &["commit", "--quiet", "-am", "Second revision"],
        );
        fixture
    }

    fn git(&self, directory: &Path, args: &[&str]) -> String {
        let result = Command::new("git")
            .current_dir(directory)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.config)
            .env("GIT_CONFIG_COUNT", "0")
            .args([
                "-c",
                "user.name=JustPaste Test",
                "-c",
                "user.email=justpaste@example.test",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.templateDir=",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Git fixture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().to_owned()
    }

    fn command(&self) -> Command {
        let local = self.source.to_string_lossy().replace('\\', "/");
        let local = if cfg!(windows) {
            format!("file:///{local}")
        } else {
            format!("file://{local}")
        };
        let mut command = Command::new(binary());
        // Exercise an HTTPS-shaped input with a real local Git transport and
        // isolated config. No server, credentials, or live clipboard required.
        command
            .args(["paste", "--output"])
            .arg(&self.output)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.config)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", format!("url.{local}.insteadOf"))
            .env("GIT_CONFIG_VALUE_0", self.remote)
            .env("JUSTTOOLS_DEFAULTS", self.temp.path().join("defaults.toml"))
            .env("YTDLP_BIN", self.temp.path().join("missing-ytdlp"))
            .env("FFMPEG_BIN", self.temp.path().join("missing-ffmpeg"))
            .env("JUSTTOOLS_NO_DOWNLOAD", "1");
        command
    }

    fn verify(&self, checkout: &Path, remote: &str) {
        assert!(checkout.join(".git").is_dir());
        assert_eq!(
            fs::read_to_string(checkout.join("README.md")).unwrap(),
            "second revision\n"
        );
        assert_eq!(self.git(checkout, &["rev-list", "--count", "HEAD"]), "2");
        assert_eq!(
            self.git(checkout, &["rev-parse", "--is-shallow-repository"]),
            "false"
        );
        assert_eq!(
            self.git(checkout, &["config", "--get", "remote.origin.url"]),
            remote
        );
        assert_eq!(self.git(checkout, &["status", "--porcelain"]), "");
        assert!(
            self.git(checkout, &["branch", "--remotes"])
                .contains("origin/other-branch")
        );
        assert_eq!(self.git(checkout, &["tag", "--list"]), "fixture-tag");
    }
}

#[test]
fn repository_link_clones_full_history_and_numbers_folder_collisions() {
    let fixture = GitFixture::new();
    fs::create_dir(&fixture.output).unwrap();
    fs::create_dir(fixture.output.join("repo.name")).unwrap();
    fs::write(fixture.output.join("repo.name (2)"), "keep existing file").unwrap();
    for (number, input) in [
        (3, fixture.remote),
        (4, "HTTPS://github.com/justpaste-fixture/repo.name"),
    ] {
        let result = fixture.command().args([input, input]).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("1 cloned"));
        fixture.verify(
            &fixture.output.join(format!("repo.name ({number})")),
            fixture.remote,
        );
    }
    assert_eq!(
        fs::read_dir(fixture.output.join("repo.name"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read_to_string(fixture.output.join("repo.name (2)")).unwrap(),
        "keep existing file"
    );
    assert_eq!(
        fs::read_to_string(fixture.source.join("README.md")).unwrap(),
        "second revision\n"
    );
    assert!(fs::read_dir(&fixture.output).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".justpaste-")
    }));
    assert!(!fixture.temp.path().join("defaults.toml").exists());
}

#[test]
fn repository_preview_needs_no_git_network_or_output_directory() {
    let fixture = GitFixture::new();
    let result = fixture
        .command()
        .env("GIT_BIN", fixture.temp.path().join("missing-git"))
        .args([
            "--dry-run",
            "https://github.com/owner/repo.name/?tab=readme#intro",
            "git@gitlab.com:group/subgroup/another.git",
            "ssh://git@forge.example/team/third.git",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let preview = String::from_utf8_lossy(&result.stdout);
    assert_eq!(preview.matches("git clone").count(), 3);
    assert!(preview.contains("repo.name"));
    assert!(preview.contains("another"));
    assert!(!preview.contains("tab=readme"));
    assert!(!fixture.output.exists());
}

#[test]
fn piped_ssh_repository_link_uses_git_and_keeps_origin() {
    use std::io::Write;
    let fixture = GitFixture::new();
    let remote = "git@github.com:justpaste-fixture/repo.name.git";
    let mut child = fixture
        .command()
        .env("GIT_CONFIG_VALUE_0", remote)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("Clone <{remote}> please.\n").as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fixture.verify(&fixture.output.join("repo.name"), remote);
}

#[test]
fn failed_clone_cleans_staging_and_does_not_stop_the_remaining_links() {
    let fixture = GitFixture::new();
    let failed = "https://forge.example/missing.git";
    let missing = fixture
        .temp
        .path()
        .join("missing-origin")
        .to_string_lossy()
        .replace('\\', "/");
    let missing = if cfg!(windows) {
        format!("file:///{missing}")
    } else {
        format!("file://{missing}")
    };
    let result = fixture
        .command()
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_1", format!("url.{missing}.insteadOf"))
        .env("GIT_CONFIG_VALUE_1", failed)
        .args([failed, fixture.remote])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Git could not clone"));
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 cloned, 1 failed"));
    fixture.verify(&fixture.output.join("repo.name"), fixture.remote);
    assert_eq!(fs::read_dir(&fixture.output).unwrap().count(), 1);
}

#[test]
fn missing_git_override_fails_without_downloading_or_creating_a_checkout() {
    let fixture = GitFixture::new();
    let result = fixture
        .command()
        .env("GIT_BIN", fixture.temp.path().join("missing-git"))
        .arg(fixture.remote)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("warning: GIT_BIN"));
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing-git"));
    assert_eq!(fs::read_dir(&fixture.output).unwrap().count(), 0);
}

#[test]
fn missing_git_on_path_warns_with_install_guidance_without_installing() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    let result = Command::new(binary())
        .args(["paste", "https://github.com/owner/repo", "--output"])
        .arg(&output)
        .env("PATH", temp.path())
        .env_remove("GIT_BIN")
        .env("JUSTTOOLS_DEFAULTS", temp.path().join("defaults.toml"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("warning: Git is not installed"));
    assert!(error.contains("https://git-scm.com/downloads"));
    assert_eq!(fs::read_dir(output).unwrap().count(), 0);
}

#[test]
fn cli_piped_text_preserves_utf8_and_preview_writes_nothing() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    let text = "piped café\nsecond line\n";
    for dry_run in [true, false] {
        let mut command = Command::new(binary());
        command
            .args(["paste", "--text", "--output"])
            .arg(&output)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if dry_run {
            command.arg("--dry-run");
        }
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if dry_run {
            assert!(!output.exists());
        }
    }
    let file = fs::read_dir(output)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read_to_string(file).unwrap(), text);
}

#[test]
#[ignore = "downloads checksum-verified vendor tools and exercises JustPaste with an empty PATH"]
fn justpaste_downloads_missing_yt_dlp_and_saves_playable_media() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    let temp = tempfile::tempdir().unwrap();
    let tools = temp.path().join("tools");
    let executable = |name: &str| {
        tools.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        })
    };
    // FFmpeg creates the controlled input clip; yt-dlp must still be absent
    // when JustPaste starts, and PATH contains no third-party tools.
    let fetched = Command::new(binary())
        .args(["deps", "fetch", "ffmpeg"])
        .env("JUSTTOOLS_DEPS_DIR", &tools)
        .env_remove("JUSTTOOLS_NO_DOWNLOAD")
        .output()
        .unwrap();
    assert!(
        fetched.status.success(),
        "{}",
        String::from_utf8_lossy(&fetched.stderr)
    );
    assert!(!executable("yt-dlp").exists());
    let clip = temp.path().join("fixture.mp4");
    let generated = Command::new(executable("ffmpeg"))
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=16x16:r=5",
            "-t",
            "0.2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&clip)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let media = fs::read(&clip).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/page", listener.local_addr().unwrap());
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let server = std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).ok();
                    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    let mut bytes = Vec::new();
                    let mut buffer = [0; 1024];
                    while bytes.len() < 8192 && !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                        let count = stream.read(&mut buffer).unwrap_or(0);
                        if count == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    let request = String::from_utf8_lossy(&bytes);
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    let page = b"<html><head><title>JustPaste managed download fixture</title></head><body><video src='/clip.mp4' controls></video></body></html>";
                    let (status, kind, body): (&str, &str, &[u8]) = match path {
                        "/page" => ("200 OK", "text/html", page),
                        "/clip.mp4" => ("200 OK", "video/mp4", &media),
                        _ => ("404 Not Found", "text/plain", b"missing"),
                    };
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).ok();
                    if !request.starts_with("HEAD ") {
                        stream.write_all(body).ok();
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    struct Stop(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
            if let Some(server) = self.1.take() {
                let _ = server.join();
            }
        }
    }
    let _stop = Stop(stopped, Some(server));
    let output = temp.path().join("downloads");
    let empty_path = temp.path().join("empty-path");
    fs::create_dir(&empty_path).unwrap();
    let result = Command::new(binary())
        .args(["paste", &url, "--output"])
        .arg(&output)
        .env("PATH", &empty_path)
        .env("JUSTTOOLS_DEPS_DIR", &tools)
        .env(
            "JUSTTOOLS_DEFAULTS",
            temp.path().join("state/defaults.toml"),
        )
        .env_remove("YTDLP_BIN")
        .env_remove("FFMPEG_BIN")
        .env_remove("JUSTTOOLS_NO_DOWNLOAD")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("yt-dlp is not installed; downloading")
    );
    assert!(executable("yt-dlp").is_file());
    let version = Command::new(executable("yt-dlp"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    let saved: Vec<_> = fs::read_dir(&output)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].extension().unwrap(), "mp4");
    let playable = Command::new(executable("ffmpeg"))
        .args(["-v", "error", "-i"])
        .arg(&saved[0])
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(
        playable.status.success(),
        "{}",
        String::from_utf8_lossy(&playable.stderr)
    );
    assert!(clip.is_file());
}

#[cfg(windows)]
#[test]
#[ignore = "requires a copied PNG image; reads but never replaces clipboard contents"]
fn live_clipboard_png_is_saved_exactly_without_changing_clipboard() {
    use windows_sys::Win32::System::{DataExchange::*, Memory::*};
    fn snapshot() -> (Vec<u8>, u32) {
        unsafe {
            assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
            struct Close;
            impl Drop for Close {
                fn drop(&mut self) {
                    unsafe {
                        CloseClipboard();
                    }
                }
            }
            let _close = Close;
            for name in ["PNG", "image/png"] {
                let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                let format = RegisterClipboardFormatW(name.as_ptr());
                let data = GetClipboardData(format);
                if data.is_null() {
                    continue;
                }
                let size = GlobalSize(data);
                assert!(size > 0 && size <= 256 * 1024 * 1024);
                let pointer = GlobalLock(data) as *const u8;
                assert!(!pointer.is_null());
                let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
                GlobalUnlock(data);
                return (bytes, GetClipboardSequenceNumber());
            }
            panic!("copy a PNG image before running this test");
        }
    }
    let (original, sequence) = snapshot();
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(binary())
        .args(["paste", "--clipboard", "--output"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let files: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(fs::read(&files[0]).unwrap(), original);
    let (after, after_sequence) = snapshot();
    assert_eq!(after_sequence, sequence, "clipboard changed during paste");
    assert_eq!(after, original);
}

fn paste(source: &Path, output: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(["paste", "--copy"])
        .arg(source)
        .arg("--output")
        .arg(output)
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn headless_filtered_copy_preview_execution_and_numbered_collision() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project with spaces");
    let output = temp.path().join("destination");
    fs::create_dir_all(source.join("node_modules/pkg")).unwrap();
    fs::create_dir_all(source.join("build/temp")).unwrap();
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(source.join(".gitignore"), "*.log\n!keep.log\n").unwrap();
    fs::write(source.join("nested/.gitignore"), "!debug.log\n").unwrap();
    for file in [
        "hello.txt",
        "keep.log",
        "debug.log",
        "nested/debug.log",
        "node_modules/pkg/x.js",
        "build/temp/x",
    ] {
        fs::write(source.join(file), file).unwrap();
    }
    let preview = paste(
        &source,
        &output,
        &[
            "--exclude",
            "node_modules;build/temp",
            "--respect-gitignore",
            "--dry-run",
        ],
    );
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    assert!(!String::from_utf8_lossy(&preview.stdout).contains("pkg/x.js"));
    for _ in 0..2 {
        let result = paste(
            &source,
            &output,
            &[
                "--exclude",
                "node_modules;build/temp",
                "--respect-gitignore",
            ],
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for name in ["project with spaces", "project with spaces (2)"] {
        let copy = output.join(name);
        assert!(copy.join("hello.txt").is_file());
        assert!(copy.join("keep.log").is_file());
        assert!(copy.join("nested/debug.log").is_file());
        assert!(!copy.join("debug.log").exists());
        assert!(!copy.join("node_modules").exists());
        assert!(!copy.join("build/temp").exists());
    }
    assert!(source.join("node_modules/pkg/x.js").is_file());
    assert!(source.join("debug.log").is_file());
}

#[test]
fn ignored_explicit_roots_and_self_copies_are_checked_before_writes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let output = temp.path().join("out");
    fs::create_dir_all(source.join("ignored")).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(source.join(".gitignore"), "ignored/\n*.log\n").unwrap();
    fs::write(source.join("debug.log"), "log").unwrap();
    fs::write(source.join("ignored/secret.txt"), "fixture").unwrap();
    for path in [source.join("ignored"), source.join("debug.log")] {
        let result = paste(&path, &output, &["--respect-gitignore"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    }
    assert!(!paste(&source, &source, &[]).status.success());
    assert!(
        !paste(&source, &output, &["--exclude", "../outside"])
            .status
            .success()
    );
}

#[test]
fn explicit_headless_copy_does_not_inherit_saved_filters() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("node_modules");
    let output = temp.path().join("out");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(source.join("x"), "fixture").unwrap();
    let defaults = temp.path().join("defaults.toml");
    fs::write(
        &defaults,
        "version = 1\n[tools.justpaste]\nignore_node_modules = 'true'\n",
    )
    .unwrap();
    for saved in [true, false] {
        let mut command = Command::new(binary());
        command
            .env("JUSTTOOLS_DEFAULTS", &defaults)
            .args(["paste", "--copy"])
            .arg(&source)
            .arg("-o")
            .arg(&output);
        if saved {
            command.arg("--saved-settings");
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(output.join("node_modules/x").exists(), !saved);
    }
}

#[cfg(windows)]
#[test]
#[ignore = "uses the Windows clipboard briefly; snapshots and restores its existing formats"]
fn windows_clipboard_png_and_dib_to_headless_file() {
    use windows::{
        Win32::{
            Foundation::*,
            System::{DataExchange::*, Memory::*, Ole::*},
            UI::WindowsAndMessaging::*,
        },
        core::w,
    };
    struct Restore {
        window: HWND,
        formats: Vec<(u32, HANDLE)>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                for _ in 0..100 {
                    if OpenClipboard(Some(self.window)).is_ok() {
                        let _ = EmptyClipboard();
                        for (format, handle) in &self.formats {
                            let _ = SetClipboardData(*format, Some(*handle));
                        }
                        let _ = CloseClipboard();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                let _ = DestroyWindow(self.window);
            }
        }
    }
    unsafe {
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("JustPaste clipboard test"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
        .unwrap();
        OpenClipboard(Some(window)).unwrap();
        let mut originals = Vec::new();
        let mut format = EnumClipboardFormats(0);
        while format != 0 {
            let original = GetClipboardData(format).unwrap();
            let duplicate =
                OleDuplicateData(original, CLIPBOARD_FORMAT(format as u16), GMEM_MOVEABLE);
            assert!(
                !duplicate.is_invalid(),
                "cannot snapshot original clipboard format {format}"
            );
            originals.push((format, duplicate));
            format = EnumClipboardFormats(format);
        }
        CloseClipboard().unwrap();
        let _restore = Restore {
            window,
            formats: originals,
        };
        let set = |format, bytes: &[u8]| {
            OpenClipboard(Some(window)).unwrap();
            EmptyClipboard().unwrap();
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).unwrap();
            let target = GlobalLock(memory) as *mut u8;
            assert!(!target.is_null());
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
            let _ = GlobalUnlock(memory);
            SetClipboardData(format, Some(HANDLE(memory.0))).unwrap();
            CloseClipboard().unwrap();
        };
        let output = tempfile::tempdir().unwrap();
        let expected = image::RgbaImage::from_fn(2, 2, |x, y| {
            image::Rgba([
                x as u8 * 100,
                y as u8 * 100,
                40,
                if x == 0 { 0 } else { 128 },
            ])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        expected
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut dib = vec![0; 124 + 16];
        dib[..4].copy_from_slice(&124_u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2_i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-2_i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
        dib[16..20].copy_from_slice(&3_u32.to_le_bytes());
        for (offset, mask) in [
            (40, 0x00ff0000_u32),
            (44, 0x0000ff00),
            (48, 0x000000ff),
            (52, 0xff000000),
        ] {
            dib[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
        }
        for (index, pixel) in expected.pixels().enumerate() {
            dib[124 + index * 4..128 + index * 4]
                .copy_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        for (format, bytes) in [
            (RegisterClipboardFormatW(w!("PNG")), png.into_inner()),
            (17, dib),
        ] {
            set(format, &bytes);
            let result = Command::new(binary())
                .args(["paste", "--clipboard", "--output"])
                .arg(output.path())
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let files: Vec<_> = fs::read_dir(output.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 2);
        for file in files {
            assert_eq!(image::open(file).unwrap().to_rgba8(), expected);
        }
        let fixture = GitFixture::new();
        let text: Vec<u8> = fixture
            .remote
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        set(13, &text); // CF_UNICODETEXT
        let sequence = GetClipboardSequenceNumber();
        let result = fixture.command().arg("--clipboard").output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        fixture.verify(&fixture.output.join("repo.name"), fixture.remote);
        assert_eq!(
            GetClipboardSequenceNumber(),
            sequence,
            "paste changed the clipboard"
        );
    }
}
