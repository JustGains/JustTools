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
    "justjpg",
    "justjson",
    "justmp3",
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

    for command in COMMANDS.iter().copied().chain(["bunt", "just", "rmbg"]) {
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
fn rmbg_auto_discloses_gpu_failure_before_cpu_fallback() {
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
    assert!(
        stdout.contains("Selected provider: CPU (Auto fallback)"),
        "{stdout}"
    );
    assert!(
        stderr.contains("CUDA unavailable:") || stderr.contains("CoreML unavailable:"),
        "{stderr}"
    );
}

#[test]
fn rmbg_strict_gpu_does_not_fall_back_with_cpu_runtime() {
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

    assert_eq!(result.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!stdout.contains("Selected provider: CPU"), "{stdout}");
    assert!(stderr.contains("check failed"), "{stderr}");
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

    for tool in ["crop", "jpg"] {
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
