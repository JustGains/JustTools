use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use zip::ZipArchive;

use crate::error::{ToolError, ToolResult};

use super::Provider;

const TOOL: &str = "justrmbg";
const MODEL_NAME: &str = "rmbg-2.0.onnx";
const MODEL_URL: &str = "https://m.justgains.com/tools/rmbg-2.0.zip";
const ARCHIVE_SHA256: &str = "03c1978e7838e997601c81cd89e912403a322a63a49a2bfe93497d24be35aad8";
const MODEL_SHA256: &str = "5b486f08200f513f460da46dd701db5fbb47d79b4be4b708a19444bcd4e79958";
const MAX_ARCHIVE_BYTES: u64 = 1_100_000_000;
const MAX_MODEL_BYTES: u64 = 1_200_000_000;
/// The managed model in half precision, made with onnxconverter-common 1.16.0
/// and onnx 1.23.2 (see docs/rmbg.md). It is never downloaded.
const HALF_MODEL_NAME: &str = "rmbg-2.0-fp16.onnx";
const HALF_MODEL_SHA256: &str = "3f281cbac29e7a95eddd424ef565ea3cff88fe75546fe09901bd571fcdb3e2d4";
const MAX_HALF_MODEL_BYTES: u64 = 700_000_000;

pub fn resolve(explicit: Option<&Path>, download_approved: bool) -> ToolResult<PathBuf> {
    if let Some(explicit) = explicit {
        return require_override(explicit, "--model");
    }
    let env_model = env::var_os("RMBG_MODEL")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Some(env_model) = env_model {
        return require_override(&env_model, "RMBG_MODEL");
    }
    let sibling_model = env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("models").join(MODEL_NAME))
    });
    let cache_model = cache_model_path()?;
    let candidates = [sibling_model, Some(cache_model.clone())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for candidate in &candidates {
        if !candidate.is_file() {
            continue;
        }
        match verify_existing_model(candidate) {
            Ok(()) => return Ok(candidate.clone()),
            Err(error) if candidate == &cache_model => {
                eprintln!(
                    "{TOOL}: cached managed model failed verification and will be replaced: {}",
                    error.message()
                );
                break;
            }
            Err(error) => return Err(error),
        }
    }

    let destination = cache_model;
    if !download_approved {
        confirm_download(&destination, &candidates)?;
    } else {
        eprintln!(
            "{TOOL}: managed model is missing; --download approved verified installation to {}",
            destination.display()
        );
    }
    download_and_extract(&destination)?;
    if !destination.is_file() {
        return Err(ToolError::new(
            TOOL,
            format!("model download did not create {}", destination.display()),
        ));
    }
    Ok(destination)
}

fn require_override(path: &Path, source: &str) -> ToolResult<PathBuf> {
    if path.is_file() {
        Ok(path.to_path_buf())
    } else {
        Err(ToolError::new(
            TOOL,
            format!(
                "{source} model not found: {}; explicit model overrides are resolve-only and are never replaced or downloaded",
                path.display()
            ),
        ))
    }
}

/// The half-precision copy beside a managed model, for GPU providers: on
/// DirectML it cuts out in about half the time and memory with the same masks.
/// An explicit model (`--model`, `RMBG_MODEL`) is always used as given, and a
/// copy that fails its pinned SHA-256 is skipped with a warning.
pub fn half_precision(
    explicit: Option<&Path>,
    model: &Path,
    provider: Provider,
) -> Option<PathBuf> {
    let overridden =
        explicit.is_some() || env::var_os("RMBG_MODEL").is_some_and(|value| !value.is_empty());
    if overridden || provider == Provider::Cpu {
        return None;
    }
    verified_beside(
        model,
        HALF_MODEL_NAME,
        MAX_HALF_MODEL_BYTES,
        HALF_MODEL_SHA256,
    )
}

fn verified_beside(model: &Path, name: &str, max_bytes: u64, expected: &str) -> Option<PathBuf> {
    let path = model.with_file_name(name);
    if !path.is_file() {
        return None;
    }
    match verify_file(&path, max_bytes, expected) {
        Ok(()) => Some(path),
        Err(error) => {
            eprintln!("{TOOL}: not using {name}: {}", error.message());
            None
        }
    }
}

fn verify_existing_model(path: &Path) -> ToolResult {
    verify_file(
        path,
        MAX_MODEL_BYTES,
        &expected_hash("RMBG_MODEL_SHA256", MODEL_SHA256)?,
    )
}

fn verify_file(path: &Path, max_bytes: u64, expected: &str) -> ToolResult {
    let metadata = fs::metadata(path).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(ToolError::new(
            TOOL,
            format!("model has an invalid size: {}", path.display()),
        ));
    }
    let mut reader = BufReader::new(
        fs::File::open(path).map_err(|error| ToolError::new(TOOL, error.to_string()))?,
    );
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(ToolError::new(
            TOOL,
            format!(
                "model failed SHA-256 verification at {} (got {actual})",
                path.display()
            ),
        ));
    }
    eprintln!("{TOOL}: verified model SHA-256 at {}", path.display());
    Ok(())
}

fn cache_model_path() -> ToolResult<PathBuf> {
    ProjectDirs::from("com", "JustGains", "JustTools")
        .map(|dirs| dirs.cache_dir().join("rmbg").join(MODEL_NAME))
        .ok_or_else(|| ToolError::new(TOOL, "cannot determine the per-user cache directory"))
}

fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

fn confirm_download(destination: &Path, candidates: &[PathBuf]) -> ToolResult {
    let mut input = io::stdin().lock();
    confirm_download_with(destination, candidates, is_interactive(), &mut input)
}

fn confirm_download_with<R: BufRead>(
    destination: &Path,
    candidates: &[PathBuf],
    interactive: bool,
    input: &mut R,
) -> ToolResult {
    let looked = candidates
        .iter()
        .map(|path| format!("  {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    if !interactive {
        return Err(ToolError::new(
            TOOL,
            format!(
                "BRIA RMBG-2.0 model not found. Looked for:\n{looked}\n\
                 refusing to download the ~780 MiB archive without interactive confirmation.\n\
                 Run in a terminal to approve it, place the model at {}, or set RMBG_MODEL.",
                destination.display()
            ),
        ));
    }
    eprintln!(
        "BRIA RMBG-2.0 model not found.\n\
         Download: ~780 MiB (installs ~977 MiB)\n\
         Source:   {}\n\
         Target:   {}\n\
         License:  BRIA's weights require separate commercial licensing.\n\
         Download now? [y/N]",
        model_url(),
        destination.display()
    );
    let mut answer = String::new();
    input
        .read_line(&mut answer)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Err(ToolError::new(
            TOOL,
            "model download cancelled; no files were changed",
        ));
    }
    Ok(())
}

fn model_url() -> String {
    env::var("RMBG_MODEL_URL").unwrap_or_else(|_| MODEL_URL.to_owned())
}

fn expected_hash(variable: &str, default: &str) -> ToolResult<String> {
    let hash = env::var(variable)
        .unwrap_or_else(|_| default.to_owned())
        .to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ToolError::new(
            TOOL,
            format!("{variable} must contain 64 hexadecimal characters"),
        ));
    }
    Ok(hash)
}

fn download_and_extract(destination: &Path) -> ToolResult {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut archive_file = tempfile::Builder::new()
        .prefix("rmbg-2.0-")
        .suffix(".zip.partial")
        .tempfile_in(parent)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let url = model_url();
    eprintln!("{TOOL}: downloading model archive from {url} ...");
    let archive_hash = download(&url, &mut archive_file)?;
    let expected_archive = expected_hash("RMBG_MODEL_ARCHIVE_SHA256", ARCHIVE_SHA256)?;
    if archive_hash != expected_archive {
        return Err(ToolError::new(
            TOOL,
            format!(
                "model archive failed SHA-256 verification (got {archive_hash}, expected {expected_archive})"
            ),
        ));
    }
    extract_model(
        archive_file,
        destination,
        &expected_hash("RMBG_MODEL_SHA256", MODEL_SHA256)?,
    )?;
    eprintln!("{TOOL}: model installed at {}", destination.display());
    Ok(())
}

fn download(url: &str, destination: &mut NamedTempFile) -> ToolResult<String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|error| ToolError::new(TOOL, format!("model download failed: {error}")))?;
    let announced = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if announced.is_some_and(|length| length > MAX_ARCHIVE_BYTES) {
        return Err(ToolError::new(
            TOOL,
            "server announced an unexpectedly large model archive",
        ));
    }
    let mut reader = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut received = 0_u64;
    let mut next_progress = 100 * 1024 * 1024;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > MAX_ARCHIVE_BYTES {
            return Err(ToolError::new(
                TOOL,
                "model archive exceeded its safety limit",
            ));
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        hasher.update(&buffer[..count]);
        if received >= next_progress {
            eprintln!("{TOOL}:   {} MiB", received / 1024 / 1024);
            next_progress += 100 * 1024 * 1024;
        }
    }
    if announced.is_some_and(|length| length != received) {
        return Err(ToolError::new(
            TOOL,
            format!("incomplete model download: received {received} bytes"),
        ));
    }
    destination
        .flush()
        .and_then(|_| destination.as_file().sync_all())
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn extract_model(archive_file: NamedTempFile, destination: &Path, expected: &str) -> ToolResult {
    let reopened = archive_file
        .reopen()
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut archive = ZipArchive::new(reopened)
        .map_err(|error| ToolError::new(TOOL, format!("invalid model ZIP: {error}")))?;
    let index = (0..archive.len())
        .find(|&index| {
            archive
                .by_index(index)
                .ok()
                .and_then(|entry| {
                    Path::new(entry.name())
                        .file_name()
                        .map(|name| name == MODEL_NAME)
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| ToolError::new(TOOL, "model ZIP does not contain rmbg-2.0.onnx"))?;
    let mut entry = archive
        .by_index(index)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    if entry.is_dir() || entry.size() == 0 || entry.size() > MAX_MODEL_BYTES {
        return Err(ToolError::new(
            TOOL,
            "model ZIP contains an invalid model entry",
        ));
    }
    let mut output = AtomicWriteFile::open(destination)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut extracted = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = entry
            .read(&mut buffer)
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if count == 0 {
            break;
        }
        extracted += count as u64;
        if extracted > MAX_MODEL_BYTES {
            return Err(ToolError::new(
                TOOL,
                "extracted model exceeded its safety limit",
            ));
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        hasher.update(&buffer[..count]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(ToolError::new(
            TOOL,
            format!("extracted model failed SHA-256 verification (got {actual})"),
        ));
    }
    output
        .flush()
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    output
        .commit()
        .map_err(|error| ToolError::new(TOOL, format!("cannot install model: {error}")))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    #[test]
    fn half_precision_is_only_for_gpus_with_a_managed_model() {
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join(MODEL_NAME);
        fs::write(&model, b"full").unwrap();
        assert_eq!(half_precision(None, &model, Provider::DirectMl), None);
        fs::write(
            model.with_file_name(HALF_MODEL_NAME),
            b"not the pinned model",
        )
        .unwrap();
        for (explicit, provider) in [
            (None, Provider::DirectMl),
            (None, Provider::Cpu),
            (Some(model.as_path()), Provider::DirectMl),
        ] {
            assert_eq!(half_precision(explicit, &model, provider), None);
        }
    }

    #[test]
    fn a_copy_beside_the_model_is_used_only_when_verified() {
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join(MODEL_NAME);
        let bytes = b"half precision test model";
        let hash = format!("{:x}", Sha256::digest(bytes));
        assert_eq!(verified_beside(&model, "half.onnx", 100, &hash), None);
        let half = directory.path().join("half.onnx");
        fs::write(&half, bytes).unwrap();
        assert_eq!(verified_beside(&model, "half.onnx", 100, &hash), Some(half));
        assert_eq!(
            verified_beside(&model, "half.onnx", 100, MODEL_SHA256),
            None
        );
        assert_eq!(verified_beside(&model, "half.onnx", 10, &hash), None);
    }

    #[test]
    fn pinned_hashes_are_well_formed() {
        for hash in [ARCHIVE_SHA256, MODEL_SHA256, HALF_MODEL_SHA256] {
            assert_eq!(hash.len(), 64);
            assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn extraction_is_verified_and_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let mut archive = NamedTempFile::new_in(directory.path()).unwrap();
        let bytes = b"small offline test model";
        {
            let mut writer = ZipWriter::new(archive.as_file_mut());
            writer
                .start_file(
                    "safe/rmbg-2.0.onnx",
                    SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(bytes).unwrap();
            writer.finish().unwrap();
        }
        let expected = format!("{:x}", Sha256::digest(bytes));
        let destination = directory.path().join(MODEL_NAME);
        extract_model(archive, &destination, &expected).unwrap();
        assert_eq!(fs::read(destination).unwrap(), bytes);
    }

    #[test]
    fn noninteractive_model_install_makes_no_directories() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("new/models/rmbg-2.0.onnx");
        let mut input = Cursor::new(b"yes\n");
        let error = confirm_download_with(&destination, &[], false, &mut input).unwrap_err();
        assert!(error.message().contains("refusing to download"));
        assert!(!destination.parent().unwrap().exists());
    }

    #[test]
    fn declined_model_install_makes_no_directories() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("new/models/rmbg-2.0.onnx");
        let mut input = Cursor::new(b"n\n");
        let error = confirm_download_with(&destination, &[], true, &mut input).unwrap_err();
        assert!(error.message().contains("cancelled"));
        assert!(!destination.parent().unwrap().exists());
    }

    #[test]
    fn explicit_model_override_is_resolve_only_even_with_download_permission() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("custom.onnx");
        let error = resolve(Some(&missing), true).unwrap_err();
        assert!(error.message().contains("resolve-only"));
        assert!(!missing.exists());
    }
}
