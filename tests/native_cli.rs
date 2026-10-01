use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::thread;

const COMMANDS: &[&str] = &[
    "justaudio",
    "justavif",
    "justbunt",
    "justcommit",
    "justcrop",
    "justip",
    "justjpg",
    "justjson",
    "justlinks",
    "justmkcd",
    "justmp3",
    "justoptimize",
    "justpaste",
    "justpdf",
    "justpng",
    "justport",
    "justports",
    "justqr",
    "justready",
    "justresize",
    "justrmbg",
    "justsvg",
    "justvideo",
    "justwav",
    "justwebp",
    "justzip",
];

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_just"))
}

fn run(args: &[&str]) -> Output {
    Command::new(binary()).args(args).output().unwrap()
}

#[test]
fn links_dispatch_normalizes_youtube_and_casing() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("links-source.txt");
    fs::write(&source, "HTTPS://EXAMPLE.TEST/Path\nhttps://example.test/path\nhttps://youtu.be/AbC_dEf-123?si=share\nhttps://youtube.com/live/abc_def-123?t=30\n").unwrap();
    let output = Command::new(binary())
        .args(["links", "-o", "-"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "HTTPS://EXAMPLE.TEST/Path\nhttps://www.youtube.com/watch?v=AbC_dEf-123\n"
    );
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn git(directory: &std::path::Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap()
}

fn fake_openrouter(content: &str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response_body = serde_json::to_string(&serde_json::json!({
        "choices": [{"message": {"content": content}}]
    }))
    .unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 8192];
        let expected = loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "request ended before its headers");
            request.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let header_end = header_end + 4;
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break header_end + content_length;
            }
        };
        while request.len() < expected {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "request ended before its body");
            request.extend_from_slice(&buffer[..count]);
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        )
        .unwrap();
        String::from_utf8(request).unwrap()
    });
    (format!("http://{address}/api/v1/chat/completions"), handle)
}

#[test]
fn selector_lists_and_dispatches_every_command() {
    let listing = run(&["--help"]);
    assert!(listing.status.success());
    let listing = String::from_utf8_lossy(&listing.stdout);
    for command in COMMANDS {
        assert!(listing.contains(command), "selector omitted {command}");
        let short = command.strip_prefix("just").unwrap();
        let help = run(&["help", short]);
        assert!(
            help.status.success(),
            "{command} --help failed: {}",
            String::from_utf8_lossy(&help.stderr)
        );
        assert!(
            String::from_utf8_lossy(&help.stdout).contains("Usage:"),
            "{command} help had no usage"
        );
        let version = run(&[short, "--version"]);
        assert!(version.status.success(), "{command} --version failed");
    }
}

#[test]
fn selector_reports_an_overridden_defaults_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("launcher-defaults.toml");
    let output = Command::new(binary())
        .arg("--defaults-path")
        .env("JUSTTOOLS_DEFAULTS", &path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        path.display().to_string()
    );
}

#[test]
fn install_creates_native_aliases_and_backs_up_legacy_scripts() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(
        bin.join("justqr.cmd"),
        "@echo off\r\nnode \"%~dp0just-qr.js\" %*\r\n",
    )
    .unwrap();
    fs::write(
        bin.join("just-qr.js"),
        "#!/usr/bin/env node\n// legacy JustTools QR implementation\n",
    )
    .unwrap();

    let result = Command::new(binary())
        .args(["install", "--bin-dir"])
        .arg(&bin)
        .args(["--yes", "--no-path"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    for command in COMMANDS
        .iter()
        .copied()
        .chain(["bunt", "just", "mkcd", "rmbg"])
    {
        assert!(
            bin.join(executable_name(command)).is_file(),
            "missing installed alias {command}"
        );
    }
    let backup_root = bin.join(".justtools-backups");
    let backup = fs::read_dir(&backup_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(backup.join("justqr.cmd").is_file());
    assert!(backup.join("just-qr.js").is_file());

    let alias_help = Command::new(bin.join(executable_name("justjson")))
        .arg("--help")
        .output()
        .unwrap();
    assert!(alias_help.status.success());
    assert!(String::from_utf8_lossy(&alias_help.stdout).contains("Usage:"));

    let bunt_help = Command::new(bin.join(executable_name("bunt")))
        .arg("--help")
        .output()
        .unwrap();
    assert!(bunt_help.status.success());
    assert!(String::from_utf8_lossy(&bunt_help.stdout).contains("justbunt"));
}

/// A stand-in for a public address service, so the tests never reach the
/// network. It answers `connections` requests with the same response.
fn fake_ip_service(
    status: &'static str,
    body: &'static str,
    connections: usize,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for _ in 0..connections {
            let (mut stream, _) = listener.accept().unwrap();
            // A GET has no body, so its headers are the whole request.
            // Answering before they arrive would leave the client writing
            // into a closed socket.
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0, "request ended before its headers");
                request.extend_from_slice(&buffer[..count]);
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (format!("http://{address}/"), handle)
}

#[test]
fn justip_reports_both_families_without_a_switch() {
    let (ipv4, ipv4_service) = fake_ip_service("200 OK", "203.0.113.7\n", 1);
    let (ipv6, ipv6_service) = fake_ip_service("200 OK", "2001:db8::7\n", 1);
    let output = Command::new(binary())
        .args(["ip", "--json"])
        .env("JUSTIP_IPV4_URL", &ipv4)
        .env("JUSTIP_IPV6_URL", &ipv6)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        r#"{"ipv4":"203.0.113.7","ipv6":"2001:db8::7"}"#
    );
    ipv4_service.join().unwrap();
    ipv6_service.join().unwrap();
}

#[test]
fn justip_falls_back_and_refuses_an_answer_from_the_wrong_family() {
    let (failing, failing_service) = fake_ip_service("500 Internal Server Error", "nope", 1);
    let (working, working_service) = fake_ip_service("200 OK", "203.0.113.7", 1);
    // The IPv6 service answers with an IPv4 address, which must be discarded
    // rather than reported under the wrong label.
    let (confused, confused_service) = fake_ip_service("200 OK", "203.0.113.7", 1);
    let output = Command::new(binary())
        .args(["ip", "--plain"])
        .env("JUSTIP_IPV4_URL", format!("{failing};{working}"))
        .env("JUSTIP_IPV6_URL", &confused)
        .output()
        .unwrap();
    // One family answering is still an answer, so the run succeeds.
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "203.0.113.7\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("IPv6 unavailable"), "{stderr}");
    assert!(stderr.contains("wrong address family"), "{stderr}");
    failing_service.join().unwrap();
    working_service.join().unwrap();
    confused_service.join().unwrap();
}

#[test]
fn justip_fails_when_no_requested_family_answers() {
    let (failing, failing_service) = fake_ip_service("500 Internal Server Error", "nope", 1);
    let output = Command::new(binary())
        .args(["ip", "-4"])
        .env("JUSTIP_IPV4_URL", &failing)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no public address available"), "{stderr}");
    failing_service.join().unwrap();
}

#[test]
fn bunt_snapshot_runs_through_short_dispatch() {
    let snapshot = run(&["bunt", "--snapshot"]);
    assert!(
        snapshot.status.success(),
        "bunt snapshot failed: {}",
        String::from_utf8_lossy(&snapshot.stderr)
    );
    let stdout = String::from_utf8_lossy(&snapshot.stdout);
    assert!(stdout.contains("STATE"));
    assert!(stdout.contains("RUNTIME"));
    assert!(stdout.contains("WORKLOAD"));
}

#[test]
fn justports_snapshot_and_json_run_through_short_dispatch() {
    let directory = tempfile::tempdir().unwrap();
    let history = directory.path().join("ports-history.json");
    let snapshot = Command::new(binary())
        .args(["ports", "--snapshot", "--all"])
        .env("JUSTPORTS_HISTORY", &history)
        .output()
        .unwrap();
    assert!(
        snapshot.status.success(),
        "JustPorts snapshot failed: {}",
        String::from_utf8_lossy(&snapshot.stderr)
    );
    let stdout = String::from_utf8_lossy(&snapshot.stdout);
    assert!(
        stdout.contains("TCP listeners") || (stdout.contains("PORT") && stdout.contains("URL")),
        "{stdout}"
    );

    let json = Command::new(binary())
        .args(["ports", "--json", "--all"])
        .env("JUSTPORTS_HISTORY", &history)
        .output()
        .unwrap();
    assert!(
        json.status.success(),
        "JustPorts JSON failed: {}",
        String::from_utf8_lossy(&json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(value.is_array());
    if let Some(server) = value.as_array().and_then(|servers| servers.first()) {
        assert!(server["port"].is_number());
        assert!(server["url"].is_string());
        assert!(server["projectName"].is_string());
    }
}

#[test]
fn justcommit_stages_by_default_uses_rules_and_pushes_the_created_commit() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path();
    let remote_directory = tempfile::tempdir().unwrap();
    let remote = remote_directory.path().join("origin.git");
    assert!(
        git(
            remote_directory.path(),
            &["init", "--bare", "--quiet", "origin.git"]
        )
        .status
        .success()
    );
    assert!(git(repository, &["init", "--quiet"]).status.success());
    assert!(
        git(repository, &["config", "user.name", "JustCommit Test"])
            .status
            .success()
    );
    assert!(
        git(
            repository,
            &["config", "user.email", "justcommit@example.invalid"]
        )
        .status
        .success()
    );
    fs::write(repository.join("README.md"), "# Test repository\n").unwrap();
    assert!(git(repository, &["add", "README.md"]).status.success());
    assert!(
        git(repository, &["commit", "--quiet", "-m", "Initial commit"])
            .status
            .success()
    );
    assert!(
        git(
            repository,
            &["remote", "add", "origin", remote.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(
        git(
            repository,
            &["push", "--quiet", "--set-upstream", "origin", "HEAD"]
        )
        .status
        .success()
    );
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::create_dir_all(repository.join(".cursor/rules")).unwrap();
    fs::write(
        repository.join("src/greeting.rs"),
        "pub fn greeting() -> &'static str { \"hello\" }\n",
    )
    .unwrap();
    fs::write(
        repository.join(".cursor/rules/git-commit-structure.mdc"),
        "Use type(scope): subject and explain user impact.",
    )
    .unwrap();

    let generated = serde_json::json!({
        "summary": "Add a reusable greeting helper",
        "message": "feat(core): add greeting helper\n\nExpose a small reusable greeting for callers."
    })
    .to_string();
    let (url, server) = fake_openrouter(&generated);
    let result = Command::new(binary())
        .args(["commit", "--directory"])
        .arg(repository)
        .args([
            "--api-key",
            "integration-test-key",
            "--model",
            "test/fast-model",
            "--push",
        ])
        .env("JUSTCOMMIT_OPENROUTER_URL", url)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justcommit failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout).replace("\r\n", "\n");
    assert!(stdout.contains("Summary: Add a reusable greeting helper"));
    assert!(stdout.contains("feat(core): add greeting helper"));
    let success = stdout
        .split_once("justcommit: committed")
        .expect("successful output should identify the created commit")
        .1;
    assert!(success.contains(
        "Commit message:\nfeat(core): add greeting helper\n\nExpose a small reusable greeting for callers."
    ));
    assert!(stdout.contains("justcommit: pushed"));

    let request = server.join().unwrap();
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer integration-test-key")
    );
    let body = request.split_once("\r\n\r\n").unwrap().1;
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["model"], "test/fast-model");
    let prompt = body["messages"][1]["content"].as_str().unwrap();
    assert!(prompt.contains("Use type(scope): subject and explain user impact."));
    assert!(prompt.contains("src/greeting.rs"));

    let log = git(repository, &["log", "-1", "--pretty=%B"]);
    assert!(log.status.success());
    let log = String::from_utf8_lossy(&log.stdout).replace("\r\n", "\n");
    assert_eq!(
        log.trim(),
        "feat(core): add greeting helper\n\nExpose a small reusable greeting for callers."
    );
    let remote_log = git(
        remote_directory.path(),
        &[
            "--git-dir",
            "origin.git",
            "log",
            "--all",
            "-1",
            "--pretty=%B",
        ],
    );
    assert!(remote_log.status.success());
    let remote_log = String::from_utf8_lossy(&remote_log.stdout).replace("\r\n", "\n");
    assert_eq!(remote_log.trim(), log.trim());
}

#[test]
fn justcommit_requires_an_explicit_or_environment_openrouter_key() {
    let directory = tempfile::tempdir().unwrap();
    assert!(git(directory.path(), &["init", "--quiet"]).status.success());
    fs::write(directory.path().join("change.txt"), "change\n").unwrap();
    assert!(
        git(directory.path(), &["add", "change.txt"])
            .status
            .success()
    );
    let result = Command::new(binary())
        .args(["commit", "--directory"])
        .arg(directory.path())
        .arg("--dry-run")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("OpenRouter key missing"));
}

#[test]
#[ignore = "requires OPENROUTER_API_KEY and spends a tiny amount of credit"]
fn justcommit_live_openrouter_dry_run_exercises_the_complete_digest_flow() {
    assert!(
        std::env::var("OPENROUTER_API_KEY").is_ok(),
        "OPENROUTER_API_KEY must be set for the live test"
    );
    let directory = tempfile::tempdir().unwrap();
    assert!(git(directory.path(), &["init", "--quiet"]).status.success());
    fs::create_dir_all(directory.path().join("src")).unwrap();
    fs::write(
        directory.path().join("src/hello.rs"),
        "pub fn hello() -> &'static str { \"hello\" }\n",
    )
    .unwrap();
    assert!(git(directory.path(), &["add", "--all"]).status.success());
    let result = Command::new(binary())
        .args(["commit", "--directory"])
        .arg(directory.path())
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "live justcommit failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Summary:"));
    assert!(stdout.contains("Commit message:"));
    assert!(stdout.contains("dry run; no commit created"));
    assert!(
        !git(directory.path(), &["rev-parse", "--verify", "HEAD"])
            .status
            .success()
    );
}

fn test_ort_runtime() -> Option<PathBuf> {
    let runtime = std::env::var_os("JUSTTOOLS_TEST_ORT_DYLIB_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    assert!(
        runtime.is_some() || std::env::var_os("JUSTTOOLS_REQUIRE_TEST_ORT").is_none(),
        "JUSTTOOLS_REQUIRE_TEST_ORT is set, but JUSTTOOLS_TEST_ORT_DYLIB_PATH is missing"
    );
    runtime
}

#[test]
fn rmbg_cpu_check_runs_tiny_inference_without_model_resolution() {
    let Some(runtime) = test_ort_runtime() else {
        eprintln!("skipping RMBG runtime check; JUSTTOOLS_TEST_ORT_DYLIB_PATH is not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let missing_model = directory.path().join("must-not-be-resolved.onnx");
    let result = Command::new(binary())
        .args(["rmbg", "--check", "--provider", "cpu"])
        .env("ORT_DYLIB_PATH", runtime)
        .env("RMBG_MODEL", &missing_model)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "CPU check failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Requested provider: CPU"), "{stdout}");
    assert!(stdout.contains("Selected provider: CPU"), "{stdout}");
    assert!(
        stdout.contains("session creation and inference succeeded"),
        "{stdout}"
    );
    assert!(!missing_model.exists());
}

#[test]
fn rmbg_auto_uses_gpu_or_discloses_failure_before_cpu_fallback() {
    let Some(runtime) = test_ort_runtime() else {
        eprintln!("skipping RMBG runtime check; JUSTTOOLS_TEST_ORT_DYLIB_PATH is not set");
        return;
    };
    let result = Command::new(binary())
        .args(["rmbg", "--check"])
        .env("ORT_DYLIB_PATH", runtime)
        .env(
            "RMBG_GPU_PROVIDERS",
            if cfg!(target_os = "macos") {
                "coreml"
            } else {
                "cuda"
            },
        )
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Auto check failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stdout.contains("Requested provider: Auto"), "{stdout}");
    if stdout.contains("Selected provider: CPU (Auto fallback)") {
        assert!(
            stderr.contains("CUDA unavailable:") || stderr.contains("CoreML unavailable:"),
            "{stderr}"
        );
    } else {
        let selected_gpu = if cfg!(target_os = "macos") {
            "Selected provider: CoreML"
        } else {
            "Selected provider: CUDA"
        };
        assert!(stdout.contains(selected_gpu), "{stdout}");
    }
}

#[test]
fn rmbg_strict_gpu_never_falls_back() {
    let Some(runtime) = test_ort_runtime() else {
        eprintln!("skipping RMBG runtime check; JUSTTOOLS_TEST_ORT_DYLIB_PATH is not set");
        return;
    };
    let provider = if cfg!(target_os = "macos") {
        "coreml"
    } else {
        "cuda"
    };
    let result = Command::new(binary())
        .args(["rmbg", "--check", "--provider", provider])
        .env("ORT_DYLIB_PATH", runtime)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!stdout.contains("Selected provider: CPU"), "{stdout}");
    if result.status.success() {
        let selected_gpu = if cfg!(target_os = "macos") {
            "Selected provider: CoreML"
        } else {
            "Selected provider: CUDA"
        };
        assert!(stdout.contains(selected_gpu), "{stdout}");
        assert!(
            stdout.contains("session creation and inference succeeded"),
            "{stdout}"
        );
    } else {
        assert_eq!(result.status.code(), Some(1));
        assert!(stderr.contains("check failed"), "{stderr}");
    }
}

#[test]
fn rmbg_rejects_missing_input_before_runtime_resolution() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing.png");
    let result = Command::new(binary())
        .arg("rmbg")
        .arg(&missing)
        .env_remove("ORT_DYLIB_PATH")
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("input not found"), "{stderr}");
    assert!(!stderr.contains("Refusing to download"), "{stderr}");
}

#[test]
fn rmbg_rejects_relative_runtime_override() {
    let result = Command::new(binary())
        .args(["rmbg", "--check", "--provider", "cpu"])
        .env("ORT_DYLIB_PATH", "onnxruntime.dll")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("must be an absolute path"), "{stderr}");
}

#[test]
fn missing_dependency_never_installs_without_a_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("clip.mp4");
    fs::write(
        &input,
        b"fixture is not decoded before dependency resolution",
    )
    .unwrap();
    let fake_bin = directory.path().join("fake-bin");
    fs::create_dir(&fake_bin).unwrap();

    #[cfg(windows)]
    let manager = fake_bin.join("winget.exe");
    #[cfg(target_os = "macos")]
    let manager = fake_bin.join("brew");
    #[cfg(all(unix, not(target_os = "macos")))]
    let manager = fake_bin.join("apt-get");
    fs::write(&manager, b"must not execute").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&manager, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let result = Command::new(binary())
        .arg("video")
        .arg(&input)
        .env("PATH", &fake_bin)
        // Without this the missing FFmpeg would simply be downloaded.
        .env("JUSTTOOLS_NO_DOWNLOAD", "1")
        .env_remove("FFMPEG_BIN")
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("interactive confirmation"), "{error}");
    assert_eq!(fs::read(&manager).unwrap(), b"must not execute");
    assert!(!directory.path().join("clip-web.mp4").exists());
}

#[test]
fn media_url_dry_run_reports_the_download_without_network_access() {
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(binary())
        .args(["video", "--dry-run", "https://example.test/watch?v=1"])
        .current_dir(directory.path())
        .env("PATH", directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = String::from_utf8_lossy(&result.stdout);
    assert!(report.contains("1 download(s) with yt-dlp"), "{report}");
    assert!(
        report.contains("https://example.test/watch?v=1 -> "),
        "{report}"
    );
    assert!(report.contains("<title> [<id>].mp4"), "{report}");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn image_tools_refuse_urls_instead_of_reading_them_as_paths() {
    let result = run(["png", "https://example.test/logo.png"].as_ref());
    assert_eq!(result.status.code(), Some(2));
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("reads local files"), "{error}");
}

#[test]
fn zip_uses_the_git_file_set_and_writes_a_readable_archive() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    fs::create_dir(&source).unwrap();
    let git = Command::new("git")
        .arg("init")
        .arg("--quiet")
        .arg(&source)
        .status();
    if !git.is_ok_and(|status| status.success()) {
        eprintln!("skipping ZIP integration because Git is unavailable");
        return;
    }
    fs::write(source.join("keep.txt"), "kept\n").unwrap();
    fs::write(source.join("ignored.tmp"), "ignored\n").unwrap();
    fs::write(source.join(".gitignore"), "*.tmp\n").unwrap();
    let output = directory.path().join("result.zip");

    let result = Command::new(binary())
        .arg("zip")
        .arg("--output")
        .arg(&output)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justzip failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let file = fs::File::open(output).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert!(archive.by_name("keep.txt").is_ok());
    assert!(archive.by_name(".gitignore").is_ok());
    assert!(archive.by_name("ignored.tmp").is_err());
}

#[test]
fn resize_preserves_aspect_ratio_and_keeps_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("wide.png");
    let output = directory.path().join("resized");
    image::RgbaImage::from_pixel(400, 200, image::Rgba([20, 80, 160, 200]))
        .save(&input)
        .unwrap();

    let result = Command::new(binary())
        .arg("resize")
        .arg(&input)
        .args(["--width", "100", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justresize failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(input.is_file());
    assert_eq!(
        image::image_dimensions(output.join("wide.png")).unwrap(),
        (100, 50)
    );
}

#[test]
fn crop_trims_to_the_nontransparent_alpha_bounds() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("padded.png");
    let output = directory.path().join("cropped");
    let mut image = image::RgbaImage::from_pixel(100, 80, image::Rgba([0, 0, 0, 0]));
    for y in 10..60 {
        for x in 20..70 {
            image.put_pixel(x, y, image::Rgba([20, 80, 160, 255]));
        }
    }
    image.save(&input).unwrap();

    let result = Command::new(binary())
        .arg("crop")
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justcrop failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(input.is_file());
    let cropped = image::open(output.join("padded.png")).unwrap().to_rgba8();
    assert_eq!(cropped.dimensions(), (50, 50));
    assert_eq!(cropped.get_pixel(0, 0).0, [20, 80, 160, 255]);
    assert_eq!(cropped.get_pixel(49, 49).0, [20, 80, 160, 255]);
}

#[test]
fn crop_shared_bounds_keeps_frames_aligned_and_groups_by_folder() {
    let directory = tempfile::tempdir().unwrap();
    let clip_a = directory.path().join("clip-a");
    let clip_b = directory.path().join("clip-b");
    let output = directory.path().join("cropped");
    fs::create_dir_all(&clip_a).unwrap();
    fs::create_dir_all(&clip_b).unwrap();

    let mut a_first = image::RgbaImage::from_pixel(12, 10, image::Rgba([0, 0, 0, 0]));
    for y in 5..7 {
        for x in 2..4 {
            a_first.put_pixel(x, y, image::Rgba([255, 20, 10, 255]));
        }
    }
    a_first.save(clip_a.join("a-001.png")).unwrap();

    let mut a_second = image::RgbaImage::from_pixel(12, 10, image::Rgba([0, 0, 0, 0]));
    for y in 1..3 {
        for x in 8..11 {
            a_second.put_pixel(x, y, image::Rgba([10, 80, 255, 255]));
        }
    }
    a_second.save(clip_a.join("a-002.png")).unwrap();
    image::RgbaImage::from_pixel(12, 10, image::Rgba([0, 0, 0, 0]))
        .save(clip_a.join("a-003.png"))
        .unwrap();

    let mut b_first = image::RgbaImage::from_pixel(12, 10, image::Rgba([0, 0, 0, 0]));
    b_first.put_pixel(5, 4, image::Rgba([30, 220, 70, 255]));
    b_first.save(clip_b.join("b-001.png")).unwrap();
    let mut b_second = image::RgbaImage::from_pixel(12, 10, image::Rgba([0, 0, 0, 0]));
    for y in 4..6 {
        for x in 6..8 {
            b_second.put_pixel(x, y, image::Rgba([30, 220, 70, 255]));
        }
    }
    b_second.save(clip_b.join("b-002.png")).unwrap();

    let result = Command::new(binary())
        .arg("crop")
        .arg(directory.path())
        .args(["--recursive", "--shared-bounds", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "shared-bounds justcrop failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let a_first = image::open(output.join("a-001.png")).unwrap().to_rgba8();
    let a_second = image::open(output.join("a-002.png")).unwrap().to_rgba8();
    let a_empty = image::open(output.join("a-003.png")).unwrap().to_rgba8();
    assert_eq!(a_first.dimensions(), (9, 6));
    assert_eq!(a_second.dimensions(), (9, 6));
    assert_eq!(a_empty.dimensions(), (9, 6));
    assert_eq!(a_first.get_pixel(0, 4).0, [255, 20, 10, 255]);
    assert_eq!(a_second.get_pixel(6, 0).0, [10, 80, 255, 255]);
    assert!(a_empty.pixels().all(|pixel| pixel[3] == 0));

    assert_eq!(
        image::image_dimensions(output.join("b-001.png")).unwrap(),
        (3, 2)
    );
    assert_eq!(
        image::image_dimensions(output.join("b-002.png")).unwrap(),
        (3, 2)
    );
}

#[test]
fn crop_shared_bounds_rejects_mixed_canvas_sizes_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    let clip = directory.path().join("clip");
    let output = directory.path().join("cropped");
    fs::create_dir_all(&clip).unwrap();
    image::RgbaImage::from_pixel(12, 10, image::Rgba([20, 80, 160, 255]))
        .save(clip.join("frame-001.png"))
        .unwrap();
    image::RgbaImage::from_pixel(10, 10, image::Rgba([20, 80, 160, 255]))
        .save(clip.join("frame-002.png"))
        .unwrap();

    let result = Command::new(binary())
        .arg("crop")
        .arg(&clip)
        .args(["--shared-bounds", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("one oriented canvas size per folder")
    );
    assert!(!output.exists());
}

#[test]
fn crop_preserves_sixteen_bit_precision_and_tiny_nonzero_alpha() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("rgba16.png");
    let output = directory.path().join("cropped");
    let mut image = image::ImageBuffer::from_pixel(8, 6, image::Rgba([0_u16, 0, 0, 0]));
    for y in 2..5 {
        for x in 3..7 {
            image.put_pixel(x, y, image::Rgba([60_000, 30_000, 10_000, 1]));
        }
    }
    image::DynamicImage::ImageRgba16(image)
        .save(&input)
        .unwrap();

    let result = Command::new(binary())
        .arg("crop")
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "16-bit justcrop failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cropped = image::open(output.join("rgba16.png")).unwrap();
    assert_eq!(cropped.color(), image::ColorType::Rgba16);
    assert_eq!((cropped.width(), cropped.height()), (4, 3));
    assert_eq!(
        cropped.to_rgba16().get_pixel(0, 0).0,
        [60_000, 30_000, 10_000, 1]
    );
}

#[test]
fn jpg_optimizes_and_composites_transparency_onto_white() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("transparent.png");
    let output = directory.path().join("jpg");
    let mut image = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0]));
    for y in 16..48 {
        for x in 16..48 {
            image.put_pixel(x, y, image::Rgba([240, 20, 10, 255]));
        }
    }
    image.save(&input).unwrap();

    let result = Command::new(binary())
        .arg("jpg")
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justjpg failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(input.is_file());
    let encoded = fs::read(output.join("transparent.jpg")).unwrap();
    assert_eq!(&encoded[..2], &[0xff, 0xd8]);
    let decoded = image::open(output.join("transparent.jpg"))
        .unwrap()
        .to_rgb8();
    assert_eq!(decoded.dimensions(), (64, 64));
    let corner = decoded.get_pixel(2, 2).0;
    assert!(corner.iter().all(|channel| *channel > 240), "{corner:?}");
    let center = decoded.get_pixel(32, 32).0;
    assert!(
        center[0] > 200 && center[1] < 60 && center[2] < 60,
        "{center:?}"
    );
}

#[test]
fn image_tool_dry_runs_do_not_create_output_directories() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("image.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([20, 80, 160, 128]))
        .save(&input)
        .unwrap();

    for tool in ["crop", "jpg", "optimize"] {
        let output = directory.path().join(format!("{tool}-output"));
        let result = Command::new(binary())
            .arg(tool)
            .arg(&input)
            .args(["--output"])
            .arg(&output)
            .arg("--dry-run")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "just{tool} dry run failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!output.exists(), "just{tool} dry run created output");
    }
}

#[test]
fn optimize_preserves_transparency_and_reports_the_exact_output() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("transparent.png");
    let output = directory.path().join("web");
    let mut image = image::RgbaImage::from_pixel(96, 64, image::Rgba([20, 80, 160, 0]));
    for y in 8..56 {
        for x in 12..84 {
            image.put_pixel(x, y, image::Rgba([220, 40, 80, 180]));
        }
    }
    image.save(&input).unwrap();

    let result = Command::new(binary())
        .arg("optimize")
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justoptimize failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(input.is_file());
    let outputs = fs::read_dir(&output)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert_ne!(
        outputs[0]
            .path()
            .extension()
            .and_then(|value| value.to_str()),
        Some("jpg")
    );
    assert!(
        image::open(outputs[0].path())
            .unwrap()
            .to_rgba8()
            .pixels()
            .any(|pixel| pixel[3] < 255)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("transparency preserved"));
    assert!(stdout.contains(&outputs[0].path().display().to_string()));
}

#[test]
fn optimize_replace_removes_non_web_source_only_after_output_exists() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("photo.bmp");
    image::RgbImage::from_fn(128, 96, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8])
    })
    .save(&input)
    .unwrap();

    let result = Command::new(binary())
        .arg("optimize")
        .arg(&input)
        .args(["--replace", "--yes"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justoptimize --replace failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!input.exists());
    let outputs = fs::read_dir(directory.path())
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        outputs[0]
            .path()
            .extension()
            .and_then(|value| value.to_str()),
        Some("png" | "webp" | "jpg")
    ));
    assert_eq!(
        image::image_dimensions(outputs[0].path()).unwrap(),
        (128, 96)
    );
}

#[test]
fn jpg_replace_converts_then_removes_a_non_jpeg_source() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("replace-me.png");
    image::RgbImage::from_pixel(24, 12, image::Rgb([20, 80, 160]))
        .save(&input)
        .unwrap();

    let result = Command::new(binary())
        .arg("jpg")
        .arg(&input)
        .arg("--replace")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justjpg --replace failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!input.exists());
    assert_eq!(
        image::image_dimensions(directory.path().join("replace-me.jpg")).unwrap(),
        (24, 12)
    );
}

#[test]
fn jpg_replace_keeps_a_jpeg_extension_and_does_not_touch_its_sibling() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("photo.jpeg");
    let sibling = directory.path().join("photo.jpg");
    image::RgbImage::from_pixel(24, 12, image::Rgb([20, 80, 160]))
        .save(&input)
        .unwrap();
    fs::write(&sibling, b"unselected sibling").unwrap();

    let result = Command::new(binary())
        .arg("jpg")
        .arg(&input)
        .arg("--replace")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "justjpg .jpeg replacement failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(image::image_dimensions(&input).unwrap(), (24, 12));
    assert_eq!(fs::read(&sibling).unwrap(), b"unselected sibling");
}

#[cfg(windows)]
#[test]
#[allow(clippy::permissions_set_readonly_false)]
fn crop_replace_preserves_the_windows_readonly_attribute() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("readonly.png");
    let mut image = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
    image.put_pixel(4, 4, image::Rgba([20, 80, 160, 255]));
    image.save(&input).unwrap();
    let mut permissions = fs::metadata(&input).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&input, permissions).unwrap();

    let result = Command::new(binary())
        .arg("crop")
        .arg(&input)
        .arg("--replace")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "read-only justcrop failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::metadata(&input).unwrap().permissions().readonly());
    let mut permissions = fs::metadata(&input).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&input, permissions).unwrap();
}

#[cfg(windows)]
#[test]
fn jpg_replace_returns_failure_when_the_source_cannot_be_removed() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("locked.png");
    image::RgbImage::from_pixel(24, 12, image::Rgb([20, 80, 160]))
        .save(&input)
        .unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&input)
        .unwrap();

    let result = Command::new(binary())
        .arg("jpg")
        .arg(&input)
        .arg("--replace")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(input.is_file());
    assert!(directory.path().join("locked.jpg").is_file());
    assert!(String::from_utf8_lossy(&result.stderr).contains("source could not be removed"));
    drop(lock);
}

#[test]
fn jpg_output_directory_cannot_silently_overwrite_its_input() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.jpg");
    image::RgbImage::from_pixel(24, 12, image::Rgb([20, 80, 160]))
        .save(&input)
        .unwrap();
    let before = fs::read(&input).unwrap();

    let result = Command::new(binary())
        .arg("jpg")
        .arg(&input)
        .arg("--output")
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("use --replace"));
    assert_eq!(fs::read(&input).unwrap(), before);
}

#[test]
fn invalid_selector_option_uses_the_standard_usage_exit() {
    let result = run(&["--unknown"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("Try 'just --help'"));
}

#[test]
fn context_menu_entries_run_the_same_headless_command_they_show() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("wide photo.png");
    let unrelated = directory.path().join("notes.txt");
    image::RgbaImage::from_pixel(1200, 600, image::Rgba([20, 80, 160, 255]))
        .save(&image)
        .unwrap();
    fs::write(&unrelated, "not an image").unwrap();
    // Explorer hands the selection over in a list file that is consumed.
    let list = directory.path().join("selection.txt");
    fs::write(
        &list,
        format!("{}\r\n{}\r\n", image.display(), unrelated.display()),
    )
    .unwrap();

    let result = Command::new(binary())
        .args(["context", "run", "resize.1024", "--list"])
        .arg(&list)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "context run failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("Headless: justresize --max 1024 'wide photo.png'"),
        "{stdout}"
    );
    assert!(!list.exists(), "the selection list must be consumed");
    assert!(image.is_file(), "the source must be kept");
    assert_eq!(
        image::image_dimensions(directory.path().join("wide photo-resized.png")).unwrap(),
        (1024, 512)
    );
}

#[test]
fn context_menu_refuses_unknown_entries_and_selections_a_tool_cannot_read() {
    let unknown = run(&["context", "run", "video.9000", "clip.mp4"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown context-menu entry"));

    let unreadable = run(&["context", "run", "optimize", "song.mp3"]);
    assert_eq!(unreadable.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unreadable.stderr).contains("JustOptimize reads"));

    // Launcher entries are interactive, so they never run without a terminal.
    let launcher = run(&["context", "run", "video.options", "clip.mp4"]);
    assert_eq!(launcher.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&launcher.stderr).contains("needs a terminal"));

    let help = run(&["context", "--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("just context install"));
}

#[test]
fn context_package_layout_is_ready_for_signing() {
    let directory = tempfile::tempdir().unwrap();
    let layout = directory.path().join("package");
    let result = Command::new(binary())
        .args([
            "context",
            "package",
            "--publisher",
            "CN=Example",
            "--output",
        ])
        .arg(&layout)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = fs::read_to_string(layout.join("AppxManifest.xml")).unwrap();
    assert!(manifest.contains(r#"Publisher="CN=Example""#));
    assert!(manifest.contains(r#"<desktop5:ItemType Type=".mp4">"#));
    assert!(layout.join("Assets").join("logo-44.png").is_file());
}

#[test]
fn video_resolution_is_validated_before_any_file_is_touched() {
    let directory = tempfile::tempdir().unwrap();
    let clip = directory.path().join("clip.mp4");
    fs::write(&clip, b"not really a video").unwrap();
    let preview = Command::new(binary())
        .args(["video", "--resolution", "4k", "--dry-run"])
        .arg(&clip)
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains("clip-web.mp4"));

    let invalid = Command::new(binary())
        .args(["video", "--resolution", "900p", "--dry-run"])
        .arg(&clip)
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("resolution must be 480p, 720p, 1080p, 1440p, 4k, or source")
    );
}

#[test]
#[cfg(not(windows))]
fn context_menu_registration_is_windows_only() {
    let result = run(&["context", "install"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("Windows File Explorer"));
}

/// A stand-in web host for `justpaste`: `/photo` is a JPEG with no extension in
/// its address, `/post` is a page that declares that photo, and anything else
/// is missing. It serves until `requests` connections have been answered.
fn fake_paste_host(requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0, "request ended before its headers");
                request.extend_from_slice(&buffer[..count]);
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, kind, body): (&str, &str, Vec<u8>) = match path.as_str() {
                "/photo" => (
                    "200 OK",
                    "image/jpeg",
                    b"\xff\xd8\xff\xe0 pretend jpeg".to_vec(),
                ),
                "/post" => (
                    "200 OK",
                    "text/html; charset=utf-8",
                    b"<html><head><meta property=\"og:image\" content=\"/photo\"></head></html>"
                        .to_vec(),
                ),
                _ => ("404 Not Found", "text/plain", b"missing".to_vec()),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (format!("http://{address}"), handle)
}

#[test]
fn paste_saves_a_direct_file_and_numbers_a_taken_name() {
    let directory = tempfile::tempdir().unwrap();
    let (host, server) = fake_paste_host(2);
    for _ in 0..2 {
        let result = Command::new(binary())
            .args(["paste", &format!("{host}/photo")])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "justpaste failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    server.join().unwrap();
    // The address has no extension, so the content type supplies one.
    let first = fs::read(directory.path().join("photo.jpg")).unwrap();
    assert!(first.starts_with(b"\xff\xd8\xff"));
    assert_eq!(
        fs::read(directory.path().join("photo (2).jpg")).unwrap(),
        first
    );
    let leftovers: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".justpaste-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files remain: {leftovers:?}"
    );
}

#[test]
fn paste_falls_back_to_the_media_a_page_declares_when_yt_dlp_is_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("saved");
    let (host, server) = fake_paste_host(2);
    let result = Command::new(binary())
        .args(["paste", "--output"])
        .arg(&output)
        .arg(format!("{host}/post"))
        .env("YTDLP_BIN", "justtools-test-missing-yt-dlp")
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(
        result.status.success(),
        "justpaste failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(output.join("photo.jpg").is_file());
}

#[test]
fn paste_reports_a_dead_link_and_previews_without_the_network() {
    let directory = tempfile::tempdir().unwrap();
    let (host, server) = fake_paste_host(1);
    let dead = Command::new(binary())
        .args(["paste", &format!("{host}/gone")])
        .env("YTDLP_BIN", "justtools-test-missing-yt-dlp")
        .current_dir(directory.path())
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(dead.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&dead.stderr).contains("could not be fetched"));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);

    // No server is listening for this one: a dry run must not connect.
    let preview = run(&["paste", "--dry-run", "https://example.invalid/clip"]);
    assert!(preview.status.success());
    assert!(String::from_utf8_lossy(&preview.stdout).contains("https://example.invalid/clip"));

    let not_a_link = run(&["paste", "clip.mp4"]);
    assert_eq!(not_a_link.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&not_a_link.stderr).contains("http(s) link"));
}

/// Downloads the real vendor builds, so it only runs when asked for:
/// `cargo test --test native_cli managed_downloads -- --ignored`.
#[test]
#[ignore = "downloads yt-dlp and FFmpeg from their vendors"]
fn managed_downloads_fetch_verified_programs_that_run() {
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(binary())
        .args(["deps", "fetch", "yt-dlp", "ffmpeg"])
        .env("JUSTTOOLS_DEPS_DIR", directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let version = Command::new(directory.path().join(executable_name("yt-dlp")))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success(), "yt-dlp does not run");
    let encoders = Command::new(directory.path().join(executable_name("ffmpeg")))
        .args(["-hide_banner", "-encoders"])
        .output()
        .unwrap();
    let encoders = String::from_utf8_lossy(&encoders.stdout).into_owned();
    for encoder in ["libx264", "libmp3lame", "aac"] {
        assert!(encoders.contains(encoder), "FFmpeg lacks {encoder}");
    }
    let probe = Command::new(directory.path().join(executable_name("ffprobe")))
        .arg("-version")
        .output()
        .unwrap();
    assert!(probe.status.success(), "ffprobe does not run");
    let leftovers = fs::read_dir(directory.path())
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".download-")
        })
        .count();
    assert_eq!(leftovers, 0, "temporary downloads remain");
}

#[test]
fn deps_fetch_needs_a_program_it_manages() {
    let nothing = run(&["deps", "fetch"]);
    assert_eq!(nothing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&nothing.stderr).contains("just deps fetch"));
    let unmanaged = run(&["deps", "fetch", "git"]);
    assert_eq!(unmanaged.status.code(), Some(2));
}

#[test]
fn a_downloaded_program_is_used_when_path_has_none() {
    let directory = tempfile::tempdir().unwrap();
    let managed = directory.path().join("managed");
    let empty = directory.path().join("empty-path");
    fs::create_dir(&managed).unwrap();
    fs::create_dir(&empty).unwrap();
    // Stand-ins: resolving them is the point, and they fail when run.
    for name in ["ffmpeg", "ffprobe"] {
        fs::write(managed.join(executable_name(name)), b"stand-in").unwrap();
    }
    let clip = directory.path().join("clip.mp4");
    fs::write(&clip, b"not decoded before the encoder is checked").unwrap();
    let result = Command::new(binary())
        .arg("video")
        .arg(&clip)
        .env("PATH", &empty)
        .env("JUSTTOOLS_DEPS_DIR", &managed)
        .env("JUSTTOOLS_NO_DOWNLOAD", "1")
        .env_remove("FFMPEG_BIN")
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    // Reaching the stand-in proves it was resolved; no installer was proposed.
    assert!(!result.status.success());
    assert!(!error.contains("interactive confirmation"), "{error}");
    assert!(!error.contains("proposed command"), "{error}");
}
