//! Private copies of yt-dlp and FFmpeg for machines that have neither.
//!
//! When one of them is not on `PATH`, JustTools downloads the vendor's own
//! build into a per-user folder and uses it from there. Nothing is installed
//! system-wide, `PATH` is not edited, and a copy already on `PATH` always
//! wins. Every download is checked against the SHA-256 the vendor publishes
//! beside it and is moved into place only after it verifies.
//!
//! `JUSTTOOLS_NO_DOWNLOAD=1` turns this off, and `JUSTTOOLS_DEPS_DIR` moves
//! the folder.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use ureq::ResponseExt;

const YT_DLP_RELEASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
/// Gyan Doshi's release build, the same one WinGet's `Gyan.FFmpeg` installs.
const FFMPEG_WINDOWS: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
/// Martin Riedl's static release builds, one zip per program.
const FFMPEG_UNIX: &str = "https://ffmpeg.martin-riedl.de/redirect/latest";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Program {
    YtDlp,
    Ffmpeg,
}

impl Program {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "yt-dlp" => Some(Self::YtDlp),
            "ffmpeg" | "ffprobe" => Some(Self::Ffmpeg),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Ffmpeg => "FFmpeg",
        }
    }
}

/// Whether automatic downloads are allowed in this environment.
pub fn enabled() -> bool {
    !std::env::var("JUSTTOOLS_NO_DOWNLOAD").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// The folder that holds the downloaded programs.
pub fn directory() -> Option<PathBuf> {
    if let Some(directory) =
        std::env::var_os("JUSTTOOLS_DEPS_DIR").filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(directory));
    }
    directories::BaseDirs::new().map(|base| base.data_local_dir().join("JustTools/deps/bin"))
}

fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// A previously downloaded program, by its plain name.
pub fn find(name: &str) -> Option<PathBuf> {
    Program::from_name(name)?;
    let path = directory()?.join(executable(name));
    path.is_file().then_some(path)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .user_agent(concat!("JustTools/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    agent
        .get(url)
        .call()
        .and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(1024 * 1024)
                .read_to_string()
        })
        .map_err(|error| format!("cannot read {url}: {error}"))
}

/// The hash a checksum listing gives for `name`. A listing that holds a lone
/// hash, as a `.sha256` sidecar often does, names whatever it sits beside.
fn listed_hash(listing: &str, name: &str) -> Option<String> {
    let valid = |hash: &str| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    let mut lone = None;
    for line in listing.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next().filter(|hash| valid(hash)) else {
            continue;
        };
        match parts.next() {
            Some(file) if file.trim_start_matches('*') == name => {
                return Some(hash.to_ascii_lowercase());
            }
            Some(_) => {}
            None => lone = Some(hash.to_ascii_lowercase()),
        }
    }
    lone
}

/// Download `url` into `directory` under a temporary name, returning the file,
/// its SHA-256, and the address it finally came from.
fn download(
    agent: &ureq::Agent,
    url: &str,
    directory: &Path,
) -> Result<(tempfile::NamedTempFile, String, String), String> {
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| format!("cannot download {url}: {error}"))?;
    let source = response.get_uri().to_string();
    let mut file = tempfile::Builder::new()
        .prefix(".download-")
        .tempfile_in(directory)
        .map_err(|error| format!("cannot write in {}: {error}", directory.display()))?;
    let mut hasher = Sha256::new();
    let mut reader = response.body_mut().as_reader();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        if crate::common::interrupted() {
            return Err("interrupted".into());
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("the download of {url} stopped: {error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        file.write_all(&buffer[..count])
            .map_err(|error| format!("cannot write the download: {error}"))?;
    }
    file.flush()
        .map_err(|error| format!("cannot write the download: {error}"))?;
    let hash = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((file, hash, source))
}

fn verify(actual: &str, listing: &str, name: &str, url: &str) -> Result<(), String> {
    match listed_hash(listing, name) {
        Some(expected) if expected == actual => Ok(()),
        Some(expected) => Err(format!(
            "{url} failed verification: expected SHA-256 {expected}, received {actual}"
        )),
        None => Err(format!("the vendor published no SHA-256 for {name}")),
    }
}

/// Put a finished file in place and make it runnable.
fn install(file: tempfile::NamedTempFile, destination: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o755)).map_err(|error| {
            format!("cannot mark {} executable: {error}", destination.display())
        })?;
    }
    file.persist(destination)
        .map(drop)
        .map_err(|error| format!("cannot install {}: {}", destination.display(), error.error))
}

/// Copy the named programs out of a zip archive, wherever they sit in it.
fn unpack(archive: &Path, programs: &[&str], directory: &Path) -> Result<(), String> {
    let broken =
        |error: &dyn std::fmt::Display| format!("the FFmpeg archive is unreadable: {error}");
    let file = File::open(archive).map_err(|error| broken(&error))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| broken(&error))?;
    for program in programs {
        let wanted = executable(program);
        let index = (0..zip.len())
            .find(|index| {
                zip.by_index(*index).is_ok_and(|entry| {
                    entry.is_file()
                        && entry.name().rsplit(['/', '\\']).next() == Some(wanted.as_str())
                })
            })
            .ok_or_else(|| format!("the FFmpeg archive does not contain {wanted}"))?;
        let mut entry = zip.by_index(index).map_err(|error| broken(&error))?;
        let mut output = tempfile::Builder::new()
            .prefix(".download-")
            .tempfile_in(directory)
            .map_err(|error| format!("cannot write in {}: {error}", directory.display()))?;
        std::io::copy(&mut entry, &mut output).map_err(|error| broken(&error))?;
        output.flush().map_err(|error| broken(&error))?;
        install(output, &directory.join(&wanted))?;
    }
    Ok(())
}

fn yt_dlp_asset() -> Result<&'static str, String> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "yt-dlp_arm64.exe",
        ("windows", _) => "yt-dlp.exe",
        ("macos", _) => "yt-dlp_macos",
        ("linux", "x86_64") => "yt-dlp_linux",
        ("linux", "aarch64") => "yt-dlp_linux_aarch64",
        (os, arch) => return Err(format!("yt-dlp publishes no build for {os} {arch}")),
    })
}

fn fetch_yt_dlp(agent: &ureq::Agent, directory: &Path) -> Result<(), String> {
    let asset = yt_dlp_asset()?;
    let url = format!("{YT_DLP_RELEASE}/{asset}");
    let listing = text(agent, &format!("{YT_DLP_RELEASE}/SHA2-256SUMS"))?;
    let (file, hash, _) = download(agent, &url, directory)?;
    verify(&hash, &listing, asset, &url)?;
    install(file, &directory.join(executable("yt-dlp")))
}

fn fetch_ffmpeg(agent: &ureq::Agent, directory: &Path) -> Result<(), String> {
    if cfg!(windows) {
        // The x64 build also runs on Windows on ARM.
        let (archive, hash, source) = download(agent, FFMPEG_WINDOWS, directory)?;
        let listing = text(agent, &format!("{source}.sha256"))?;
        let name = source.rsplit('/').next().unwrap_or_default().to_owned();
        verify(&hash, &listing, &name, &source)?;
        return unpack(archive.path(), &["ffmpeg", "ffprobe"], directory);
    }
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        other => return Err(format!("no FFmpeg build is published for {other}")),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => return Err(format!("no FFmpeg build is published for {os} {other}")),
    };
    for program in ["ffprobe", "ffmpeg"] {
        let url = format!("{FFMPEG_UNIX}/{os}/{arch}/release/{program}.zip");
        let (archive, hash, source) = download(agent, &url, directory)?;
        let listing = text(agent, &format!("{source}.sha256"))?;
        verify(&hash, &listing, &format!("{program}.zip"), &source)?;
        unpack(archive.path(), &[program], directory)?;
    }
    Ok(())
}

/// Download `program` into the managed folder, replacing any earlier copy.
pub fn fetch(tool: &str, program: Program) -> Result<PathBuf, String> {
    let directory = directory().ok_or("cannot locate a per-user data folder")?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    eprintln!(
        "{tool}: {} is not installed; downloading it to {}",
        program.label(),
        directory.display()
    );
    let agent = agent();
    match program {
        Program::YtDlp => fetch_yt_dlp(&agent, &directory)?,
        Program::Ffmpeg => fetch_ffmpeg(&agent, &directory)?,
    }
    eprintln!("{tool}: {} is ready", program.label());
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_listings_match_by_name_or_stand_alone() {
        let hash = "a".repeat(64);
        let other = "b".repeat(64);
        let listing = format!("{other}  yt-dlp\n{hash}  yt-dlp.exe\n{other} *yt-dlp_macos\n");
        assert_eq!(listed_hash(&listing, "yt-dlp.exe").as_deref(), Some(&*hash));
        assert_eq!(
            listed_hash(&listing, "yt-dlp_macos").as_deref(),
            Some(&*other)
        );
        assert_eq!(listed_hash(&listing, "yt-dlp_linux"), None);
        assert_eq!(
            listed_hash(&hash.to_ascii_uppercase(), "ffmpeg.zip").as_deref(),
            Some(&*hash)
        );
        assert_eq!(listed_hash("not a hash  ffmpeg.zip", "ffmpeg.zip"), None);

        assert!(verify(&hash, &listing, "yt-dlp.exe", "https://example.test/a").is_ok());
        let mismatch = verify(&other, &listing, "yt-dlp.exe", "https://example.test/a");
        assert!(mismatch.unwrap_err().contains("failed verification"));
    }

    #[test]
    fn only_the_two_managed_programs_are_ever_looked_up() {
        assert_eq!(Program::from_name("ffprobe"), Some(Program::Ffmpeg));
        assert_eq!(Program::from_name("yt-dlp"), Some(Program::YtDlp));
        assert_eq!(Program::from_name("git"), None);
        assert_eq!(find("git"), None);
        assert!(yt_dlp_asset().is_ok());
    }

    #[test]
    fn archives_yield_their_programs_from_any_folder() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("build.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file(
                format!("ffmpeg-9.0-build/bin/{}", executable("ffmpeg")),
                options,
            )
            .unwrap();
        writer.write_all(b"encoder").unwrap();
        writer
            .start_file(
                format!("ffmpeg-9.0-build/bin/{}", executable("ffprobe")),
                options,
            )
            .unwrap();
        writer.write_all(b"prober").unwrap();
        writer.finish().unwrap();

        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        unpack(&archive, &["ffmpeg", "ffprobe"], &bin).unwrap();
        assert_eq!(
            fs::read(bin.join(executable("ffmpeg"))).unwrap(),
            b"encoder"
        );
        assert_eq!(
            fs::read(bin.join(executable("ffprobe"))).unwrap(),
            b"prober"
        );
        let missing = unpack(&archive, &["ffplay"], &bin).unwrap_err();
        assert!(missing.contains("does not contain"));
    }
}
