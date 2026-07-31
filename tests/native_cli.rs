use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const COMMANDS: &[&str] = &[
    "justaudio",
    "justavif",
    "justcrop",
    "justjpg",
    "justjson",
    "justmp3",
    "justpdf",
    "justpng",
    "justport",
    "justqr",
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

    for command in COMMANDS.iter().copied().chain(["just", "rmbg"]) {
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
