use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use flate2::read::GzDecoder;
use ort::ep;
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::error::{ToolError, ToolResult};

use super::Device;
use super::image_pipeline::MODEL_SIZE;

const TOOL: &str = "justrmbg";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArchiveKind {
    Zip,
    Tgz,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeAsset {
    os: &'static str,
    arch: &'static str,
    version: &'static str,
    archive_name: &'static str,
    sha256: &'static str,
    bytes: u64,
    kind: ArchiveKind,
    library_name: &'static str,
    companion_name: Option<&'static str>,
}

impl RuntimeAsset {
    fn url(self) -> String {
        format!(
            "https://github.com/microsoft/onnxruntime/releases/download/v{}/{}",
            self.version, self.archive_name
        )
    }

    fn cache_key(self) -> String {
        format!("{}-{}-{}", self.version, self.os, self.arch)
    }
}

fn asset_for(os: &str, arch: &str) -> Option<RuntimeAsset> {
    match (os, arch) {
        ("windows", "x86_64") => Some(RuntimeAsset {
            os: "windows",
            arch: "x86_64",
            version: "1.24.3",
            archive_name: "onnxruntime-win-x64-1.24.3.zip",
            sha256: "4fbfb85d0e9de9bb6fb8a9866a7cb477cbad404d889b236931bf3f5d547e5f48",
            bytes: 74_397_435,
            kind: ArchiveKind::Zip,
            library_name: "onnxruntime.dll",
            companion_name: Some("onnxruntime_providers_shared.dll"),
        }),
        ("windows", "aarch64") => Some(RuntimeAsset {
            os: "windows",
            arch: "aarch64",
            version: "1.24.3",
            archive_name: "onnxruntime-win-arm64-1.24.3.zip",
            sha256: "0b9ee92bcd1f82f684b574cb413f27595e0d2f7b96ec6d02b30301f45ad313b0",
            bytes: 75_093_351,
            kind: ArchiveKind::Zip,
            library_name: "onnxruntime.dll",
            companion_name: Some("onnxruntime_providers_shared.dll"),
        }),
        ("linux", "x86_64") => Some(RuntimeAsset {
            os: "linux",
            arch: "x86_64",
            version: "1.24.3",
            archive_name: "onnxruntime-linux-x64-1.24.3.tgz",
            sha256: "4c436a280d650f8bf32c921a2bf4de7c42cc32884c51c90e47de991708bbb5a4",
            bytes: 8_150_413,
            kind: ArchiveKind::Tgz,
            library_name: "libonnxruntime.so",
            companion_name: Some("libonnxruntime_providers_shared.so"),
        }),
        ("linux", "aarch64") => Some(RuntimeAsset {
            os: "linux",
            arch: "aarch64",
            version: "1.24.3",
            archive_name: "onnxruntime-linux-aarch64-1.24.3.tgz",
            sha256: "15100fb88b4c692cdd6bf2cca5f4a26a3806cebca8136de6681e2aba4b2ea033",
            bytes: 7_166_580,
            kind: ArchiveKind::Tgz,
            library_name: "libonnxruntime.so",
            companion_name: Some("libonnxruntime_providers_shared.so"),
        }),
        ("macos", "aarch64") => Some(RuntimeAsset {
            os: "macos",
            arch: "aarch64",
            version: "1.24.3",
            archive_name: "onnxruntime-osx-arm64-1.24.3.tgz",
            sha256: "c255663d40755f84b1b86373bdb9870bb65f3a2c3d779b3d7ae31aaa00cebb4f",
            bytes: 30_381_891,
            kind: ArchiveKind::Tgz,
            library_name: "libonnxruntime.dylib",
            companion_name: None,
        }),
        // Microsoft stopped publishing an Intel-only macOS asset after this release.
        ("macos", "x86_64") => Some(RuntimeAsset {
            os: "macos",
            arch: "x86_64",
            version: "1.22.0",
            archive_name: "onnxruntime-osx-x86_64-1.22.0.tgz",
            sha256: "e4ec94a7696de74fb1b12846569aa94e499958af6ffa186022cfde16c9d617f0",
            bytes: 27_889_590,
            kind: ArchiveKind::Tgz,
            library_name: "libonnxruntime.dylib",
            companion_name: None,
        }),
        _ => None,
    }
}

fn current_asset() -> ToolResult<RuntimeAsset> {
    asset_for(env::consts::OS, env::consts::ARCH).ok_or_else(|| {
        ToolError::new(
            TOOL,
            format!(
                "automatic ONNX Runtime installation is unsupported on {} {}; set ORT_DYLIB_PATH to a compatible runtime",
                env::consts::OS,
                env::consts::ARCH
            ),
        )
    })
}

fn cache_library(asset: RuntimeAsset) -> ToolResult<PathBuf> {
    ProjectDirs::from("com", "JustGains", "JustTools")
        .map(|dirs| {
            dirs.cache_dir()
                .join("onnxruntime")
                .join(asset.cache_key())
                .join(asset.library_name)
        })
        .ok_or_else(|| ToolError::new(TOOL, "cannot determine the per-user cache directory"))
}

pub fn initialize() -> ToolResult<PathBuf> {
    if let Some(explicit) = env::var_os("ORT_DYLIB_PATH").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(explicit);
        initialize_from(&path).map_err(|error| {
            ToolError::new(
                TOOL,
                format!("cannot load ONNX Runtime from {}: {error}", path.display()),
            )
        })?;
        return Ok(path);
    }

    let asset = current_asset()?;
    let cached = cache_library(asset)?;
    let mut candidates = Vec::new();
    if let Ok(executable) = env::current_exe()
        && let Some(directory) = executable.parent()
    {
        candidates.push(directory.join(asset.library_name));
        candidates.push(directory.join("lib").join(asset.library_name));
    }
    candidates.push(cached.clone());
    // A bare name asks the operating-system loader to search its configured paths.
    candidates.push(PathBuf::from(asset.library_name));

    let mut errors = Vec::new();
    for candidate in &candidates {
        match initialize_from(candidate) {
            Ok(()) => return Ok(candidate.clone()),
            Err(error) => errors.push(format!("{}: {error}", candidate.display())),
        }
    }

    confirm_install(asset, &cached)?;
    install_runtime(asset, &cached)?;
    initialize_from(&cached).map_err(|error| {
        ToolError::new(
            TOOL,
            format!(
                "verified ONNX Runtime was installed at {} but could not be loaded: {error}\nPrevious attempts:\n  {}",
                cached.display(),
                errors.join("\n  ")
            ),
        )
    })?;
    Ok(cached)
}

fn initialize_from(path: &Path) -> Result<(), String> {
    ort::init_from(path)
        .map_err(|error| error.to_string())?
        .commit();
    Ok(())
}

fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

fn confirm_install(asset: RuntimeAsset, target: &Path) -> ToolResult {
    let mut input = io::stdin().lock();
    confirm_install_with(asset, target, is_interactive(), &mut input)
}

fn confirm_install_with<R: BufRead>(
    asset: RuntimeAsset,
    target: &Path,
    interactive: bool,
    input: &mut R,
) -> ToolResult {
    if !interactive {
        return Err(ToolError::new(
            TOOL,
            format!(
                "ONNX Runtime was not found. Refusing to download without interactive confirmation.\nRun in a terminal to approve the verified official runtime, package {} beside the executable, or set ORT_DYLIB_PATH.\nOfficial asset: {}",
                asset.library_name,
                asset.url()
            ),
        ));
    }
    eprintln!(
        "ONNX Runtime was not found.\n\
         Download: {} MiB official Microsoft CPU runtime v{}\n\
         Source:   {}\n\
         SHA-256: {}\n\
         Target:   {}\n\
         Download now? [y/N]",
        (asset.bytes + 1024 * 1024 - 1) / 1024 / 1024,
        asset.version,
        asset.url(),
        asset.sha256,
        target.display()
    );
    let mut answer = String::new();
    input
        .read_line(&mut answer)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Err(ToolError::new(
            TOOL,
            "ONNX Runtime download cancelled; no files were changed",
        ));
    }
    Ok(())
}

fn install_runtime(asset: RuntimeAsset, library: &Path) -> ToolResult {
    let directory = library.parent().ok_or_else(|| {
        ToolError::new(TOOL, format!("invalid runtime path: {}", library.display()))
    })?;
    fs::create_dir_all(directory).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut archive = tempfile::Builder::new()
        .prefix("onnxruntime-")
        .suffix(match asset.kind {
            ArchiveKind::Zip => ".zip.partial",
            ArchiveKind::Tgz => ".tgz.partial",
        })
        .tempfile_in(directory)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    eprintln!("{TOOL}: downloading {} ...", asset.url());
    download_asset(asset, &mut archive)?;

    if let Some(companion) = asset.companion_name {
        let destination = directory.join(companion);
        extract_member(asset, archive.path(), companion, &destination, false)?;
    }
    // Install the main library last, so an interrupted installation is never considered ready.
    extract_member(asset, archive.path(), asset.library_name, library, true)?;
    eprintln!("{TOOL}: ONNX Runtime installed at {}", library.display());
    Ok(())
}

fn download_asset(asset: RuntimeAsset, output: &mut NamedTempFile) -> ToolResult {
    let mut response = ureq::get(asset.url())
        .call()
        .map_err(|error| ToolError::new(TOOL, format!("runtime download failed: {error}")))?;
    let announced = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if let Some(length) = announced
        && length != asset.bytes
    {
        return Err(ToolError::new(
            TOOL,
            format!(
                "runtime server announced {length} bytes; expected {}",
                asset.bytes
            ),
        ));
    }
    let mut reader = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut received = 0_u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > asset.bytes {
            return Err(ToolError::new(
                TOOL,
                "runtime archive exceeded its pinned size",
            ));
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        hasher.update(&buffer[..count]);
    }
    if received != asset.bytes {
        return Err(ToolError::new(
            TOOL,
            format!(
                "runtime download received {received} bytes; expected {}",
                asset.bytes
            ),
        ));
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != asset.sha256 {
        return Err(ToolError::new(
            TOOL,
            format!("runtime archive failed SHA-256 verification (got {actual})"),
        ));
    }
    output
        .flush()
        .and_then(|_| output.as_file().sync_all())
        .map_err(|error| ToolError::new(TOOL, error.to_string()))
}

fn extract_member(
    asset: RuntimeAsset,
    archive: &Path,
    requested: &str,
    destination: &Path,
    main: bool,
) -> ToolResult {
    match asset.kind {
        ArchiveKind::Zip => extract_zip_member(archive, requested, destination, main),
        ArchiveKind::Tgz => extract_tgz_member(archive, requested, destination, main),
    }
}

fn is_archive_member(path: &Path, requested: &str, main: bool) -> bool {
    if path.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("lib")) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !main {
        return name == requested;
    }
    match requested {
        "onnxruntime.dll" => name == requested,
        "libonnxruntime.so" => name
            .strip_prefix("libonnxruntime.so.")
            .is_some_and(|suffix| suffix.starts_with(|character: char| character.is_ascii_digit())),
        "libonnxruntime.dylib" => name.starts_with("libonnxruntime.") && name.ends_with(".dylib"),
        _ => false,
    }
}

fn extract_zip_member(
    archive_path: &Path,
    requested: &str,
    destination: &Path,
    main: bool,
) -> ToolResult {
    let file = File::open(archive_path).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| ToolError::new(TOOL, format!("invalid runtime ZIP: {error}")))?;
    let index = (0..archive.len())
        .find(|&index| {
            archive.by_index(index).ok().is_some_and(|entry| {
                entry.is_file() && is_archive_member(Path::new(entry.name()), requested, main)
            })
        })
        .ok_or_else(|| ToolError::new(TOOL, format!("runtime archive lacks {requested}")))?;
    let mut entry = archive
        .by_index(index)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    write_atomic(destination, &mut entry)
}

fn extract_tgz_member(
    archive_path: &Path,
    requested: &str,
    destination: &Path,
    main: bool,
) -> ToolResult {
    let file = File::open(archive_path).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let entries = archive
        .entries()
        .map_err(|error| ToolError::new(TOOL, format!("invalid runtime TGZ: {error}")))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if is_archive_member(&path, requested, main) {
            return write_atomic(destination, &mut entry);
        }
    }
    Err(ToolError::new(
        TOOL,
        format!("runtime archive lacks {requested}"),
    ))
}

fn write_atomic<R: Read>(destination: &Path, reader: &mut R) -> ToolResult {
    let mut output = AtomicWriteFile::open(destination)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    io::copy(reader, &mut output).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    output
        .flush()
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    output
        .commit()
        .map_err(|error| ToolError::new(TOOL, format!("cannot install runtime: {error}")))
}

pub struct Engine {
    session: Session,
    provider: String,
}

impl Engine {
    pub fn create(model: &Path, device: Device) -> ToolResult<Self> {
        match device {
            Device::Cpu => Self::cpu(model),
            Device::Gpu => {
                let mut errors = Vec::new();
                for provider in gpu_providers() {
                    match Self::with_provider(model, &provider) {
                        Ok(engine) => return Ok(engine),
                        Err(error) => errors.push(format!("{provider}: {}", error.message())),
                    }
                }
                Err(ToolError::new(
                    TOOL,
                    format!("GPU inference session failed:\n  {}", errors.join("\n  ")),
                ))
            }
            Device::Auto => {
                for provider in gpu_providers() {
                    if let Ok(engine) = Self::with_provider(model, &provider) {
                        return Ok(engine);
                    }
                }
                Self::cpu(model)
            }
        }
    }

    pub fn cpu(model: &Path) -> ToolResult<Self> {
        let session = session_builder()?
            .commit_from_file(model)
            .map_err(|error| ToolError::new(TOOL, format!("cannot load model on CPU: {error}")))?;
        Ok(Self {
            session,
            provider: "cpu".to_owned(),
        })
    }

    fn with_provider(model: &Path, provider: &str) -> ToolResult<Self> {
        let dispatch = match provider {
            "cuda" => ep::CUDA::default().build().error_on_failure(),
            "dml" | "directml" => ep::DirectML::default().build().error_on_failure(),
            "coreml" => ep::CoreML::default().build().error_on_failure(),
            other => {
                return Err(ToolError::new(
                    TOOL,
                    format!("unknown GPU provider: {other}"),
                ));
            }
        };
        let mut builder = session_builder()?;
        if matches!(provider, "dml" | "directml") {
            builder = builder
                .with_memory_pattern(false)
                .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        }
        builder = builder
            .with_execution_providers([dispatch])
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        let session = builder.commit_from_file(model).map_err(|error| {
            ToolError::new(TOOL, format!("cannot load model with {provider}: {error}"))
        })?;
        Ok(Self {
            session,
            provider: provider.to_owned(),
        })
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn is_gpu(&self) -> bool {
        self.provider != "cpu"
    }

    pub fn infer(&mut self, chw: &[f32]) -> ToolResult<Vec<f32>> {
        let expected = (3 * MODEL_SIZE * MODEL_SIZE) as usize;
        if chw.len() != expected {
            return Err(ToolError::new(
                TOOL,
                format!(
                    "invalid model input: got {} floats; expected {expected}",
                    chw.len()
                ),
            ));
        }
        let input = Tensor::from_array((
            [1_usize, 3, MODEL_SIZE as usize, MODEL_SIZE as usize],
            chw.to_vec().into_boxed_slice(),
        ))
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        let outputs = self
            .session
            .run(ort::inputs![input])
            .map_err(|error| ToolError::new(TOOL, format!("inference failed: {error}")))?;
        if outputs.len() == 0 {
            return Err(ToolError::new(TOOL, "model returned no outputs"));
        }
        let (_, mask) = outputs[outputs.len() - 1]
            .try_extract_tensor::<f32>()
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        Ok(mask.to_vec())
    }
}

fn session_builder() -> ToolResult<ort::session::builder::SessionBuilder> {
    let builder = Session::builder().map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let builder = builder
        .with_optimization_level(GraphOptimizationLevel::All)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    builder
        .with_parallel_execution(false)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))
}

fn gpu_providers() -> Vec<String> {
    if let Ok(value) = env::var("RMBG_GPU_PROVIDERS") {
        let configured = value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .collect::<Vec<_>>();
        if !configured.is_empty() {
            return configured;
        }
    }
    if cfg!(target_os = "windows") {
        vec!["dml".into(), "cuda".into()]
    } else if cfg!(target_os = "macos") {
        vec!["coreml".into()]
    } else {
        vec!["cuda".into()]
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn official_asset_matrix_is_complete_and_pinned() {
        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
        ] {
            let asset = asset_for(os, arch).unwrap();
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert!(
                asset
                    .url()
                    .starts_with("https://github.com/microsoft/onnxruntime/")
            );
        }
        assert!(asset_for("windows", "x86").is_none());
        assert!(asset_for("freebsd", "x86_64").is_none());
    }

    #[test]
    fn noninteractive_runtime_install_makes_no_directories() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("new/cache/onnxruntime.dll");
        let mut input = Cursor::new(b"yes\n");
        let error = confirm_install_with(
            asset_for("windows", "x86_64").unwrap(),
            &target,
            false,
            &mut input,
        )
        .unwrap_err();
        assert!(error.message().contains("Refusing to download"));
        assert!(!target.parent().unwrap().exists());
    }

    #[test]
    fn declined_runtime_install_makes_no_directories() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("new/cache/onnxruntime.dll");
        let mut input = Cursor::new(b"n\n");
        let error = confirm_install_with(
            asset_for("windows", "x86_64").unwrap(),
            &target,
            true,
            &mut input,
        )
        .unwrap_err();
        assert!(error.message().contains("cancelled"));
        assert!(!target.parent().unwrap().exists());
    }

    #[test]
    fn archive_member_filter_rejects_debug_symbols_and_links() {
        assert!(is_archive_member(
            Path::new("root/lib/libonnxruntime.so.1.24.3"),
            "libonnxruntime.so",
            true
        ));
        assert!(!is_archive_member(
            Path::new("root/lib/libonnxruntime.so"),
            "libonnxruntime.so",
            true
        ));
        assert!(!is_archive_member(
            Path::new("root/foo/libonnxruntime.1.22.0.dylib.dSYM/libonnxruntime.1.22.0.dylib"),
            "libonnxruntime.dylib",
            true
        ));
    }
}
