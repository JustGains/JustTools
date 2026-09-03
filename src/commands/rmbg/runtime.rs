use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use flate2::read::GzDecoder;
use ort::ep;
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::error::{ToolError, ToolResult};

use super::Provider;
use super::image_pipeline::MODEL_SIZE;

const TOOL: &str = "justrmbg";
static INITIALIZED_RUNTIME: OnceLock<PathBuf> = OnceLock::new();

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
        format!("cpu-{}-{}-{}", self.version, self.os, self.arch)
    }
}

#[derive(Clone, Copy, Debug)]
struct PackageAsset {
    name: &'static str,
    version: &'static str,
    url: &'static str,
    sha256: &'static str,
    bytes: u64,
}

#[derive(Clone, Copy, Debug)]
struct PackageMember {
    package: usize,
    archive_path: &'static str,
    installed_name: &'static str,
    bytes: u64,
    sha256: &'static str,
    main: bool,
}

const DML_PACKAGES: [PackageAsset; 2] = [
    PackageAsset {
        name: "Microsoft.ML.OnnxRuntime.DirectML",
        version: "1.24.3",
        url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.3/microsoft.ml.onnxruntime.directml.1.24.3.nupkg",
        sha256: "0d42ecd9a672f8621d238c0edfc243b1087a10fa90d5071399e451311350996b",
        bytes: 12_459_258,
    },
    PackageAsset {
        name: "Microsoft.AI.DirectML",
        version: "1.15.4",
        url: "https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg",
        sha256: "4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9",
        bytes: 202_292_617,
    },
];

const DML_MEMBERS: [PackageMember; 7] = [
    PackageMember {
        package: 0,
        archive_path: "runtimes/win-x64/native/onnxruntime_providers_shared.dll",
        installed_name: "onnxruntime_providers_shared.dll",
        bytes: 22_040,
        sha256: "6a95b8e65633a00e90e9c6e7f2b034708b82a58de79357e4e81bf0a06ed21145",
        main: false,
    },
    PackageMember {
        package: 1,
        archive_path: "bin/x64-win/DirectML.dll",
        installed_name: "DirectML.dll",
        bytes: 18_527_776,
        sha256: "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1",
        main: false,
    },
    PackageMember {
        package: 0,
        archive_path: "LICENSE",
        installed_name: "notices/onnxruntime-LICENSE.txt",
        bytes: 1_094,
        sha256: "c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674",
        main: false,
    },
    PackageMember {
        package: 0,
        archive_path: "ThirdPartyNotices.txt",
        installed_name: "notices/onnxruntime-ThirdPartyNotices.txt",
        bytes: 331_175,
        sha256: "fb0af774b4d7cffc5b9d046f2aaeade2f37df2f80abf8033c95dfffcc77a8866",
        main: false,
    },
    PackageMember {
        package: 1,
        archive_path: "LICENSE.txt",
        installed_name: "notices/directml-LICENSE.txt",
        bytes: 10_439,
        sha256: "a05138e3a085ff60a44881eedfa58dccb03ecc1d7b1f6ae888418e8c2fec4b8d",
        main: false,
    },
    PackageMember {
        package: 1,
        archive_path: "ThirdPartyNotices.txt",
        installed_name: "notices/directml-ThirdPartyNotices.txt",
        bytes: 4_577,
        sha256: "2c95795c13ff48a58b6ed916f37901c23d964b5d9d601af422f17ad2172e7950",
        main: false,
    },
    PackageMember {
        package: 0,
        archive_path: "runtimes/win-x64/native/onnxruntime.dll",
        installed_name: "onnxruntime.dll",
        bytes: 17_328_672,
        sha256: "8fba9fd33466c722a077731d46947d009ae20756b18977bb13158a29dd93d80a",
        main: true,
    },
];

#[derive(Debug)]
pub struct RuntimeInfo {
    pub path: PathBuf,
    pub source: &'static str,
    providers: Vec<Provider>,
    attempts: Vec<String>,
    auto_cpu_fallback: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct RuntimeManifest {
    flavor: String,
    onnxruntime_version: String,
    directml_version: String,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestFile {
    path: String,
    bytes: u64,
    sha256: String,
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
                "automatic ONNX Runtime installation is unsupported on {} {}; set ORT_DYLIB_PATH to an absolute path to a compatible runtime",
                env::consts::OS,
                env::consts::ARCH
            ),
        )
    })
}

fn cache_root() -> ToolResult<PathBuf> {
    ProjectDirs::from("com", "JustGains", "JustTools")
        .map(|dirs| dirs.cache_dir().join("onnxruntime"))
        .ok_or_else(|| ToolError::new(TOOL, "cannot determine the per-user cache directory"))
}

fn cache_library(asset: RuntimeAsset) -> ToolResult<PathBuf> {
    Ok(cache_root()?
        .join(asset.cache_key())
        .join(asset.library_name))
}

fn dml_cache_library() -> ToolResult<PathBuf> {
    Ok(cache_root()?
        .join("directml-1.24.3-1.15.4-windows-x86_64")
        .join("onnxruntime.dll"))
}

pub fn initialize(provider: Provider, download_approved: bool) -> ToolResult<RuntimeInfo> {
    let requested = requested_providers(provider)?;
    if let Some(explicit) = env::var_os("ORT_DYLIB_PATH").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(explicit);
        if !path.is_absolute() {
            return Err(ToolError::new(
                TOOL,
                format!(
                    "ORT_DYLIB_PATH must be an absolute path; '{}' would allow ambiguous runtime lookup",
                    path.display()
                ),
            ));
        }
        let mut runtime = load_runtime(path, "ORT_DYLIB_PATH")?;
        runtime.providers = requested;
        return Ok(runtime);
    }

    let planned = requested.first().copied().unwrap_or(Provider::Cpu);
    if wants_managed_directml(planned) {
        if provider == Provider::Auto
            && !validate_dml_cache(&dml_cache_library()?)?
            && !download_approved
            && !is_interactive()
        {
            let mut runtime = initialize_cpu_runtime(false)?;
            runtime.providers = vec![Provider::Cpu];
            runtime.auto_cpu_fallback = true;
            runtime.attempts.push(
                "DirectML unavailable: managed runtime is not installed and noninteractive runs cannot download it; Auto selected CPU"
                    .to_owned(),
            );
            return Ok(runtime);
        }
        let mut runtime = initialize_managed_directml(download_approved)?;
        runtime.providers = vec![Provider::DirectMl];
        return Ok(runtime);
    }
    if matches!(
        planned,
        Provider::DirectMl | Provider::Cuda | Provider::CoreMl
    ) {
        let mut runtime = initialize_custom_provider_runtime(planned)?;
        runtime.providers = requested;
        return Ok(runtime);
    }

    let mut runtime = initialize_cpu_runtime(download_approved)?;
    runtime.providers = vec![Provider::Cpu];
    Ok(runtime)
}

fn wants_managed_directml(provider: Provider) -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64")) && provider == Provider::DirectMl
}

fn initialize_managed_directml(download_approved: bool) -> ToolResult<RuntimeInfo> {
    let cached = dml_cache_library()?;
    let mut errors = Vec::new();
    if validate_dml_cache(&cached)? {
        match load_runtime(cached.clone(), "managed DirectML cache") {
            Ok(runtime) => return Ok(runtime),
            Err(error) => errors.push(format!("{}: {}", cached.display(), error.message())),
        }
    } else {
        errors.push(format!(
            "{}: cache is absent or incomplete",
            cached.display()
        ));
    }

    if !download_approved {
        confirm_dml_install(&cached)?;
    }
    install_directml(&cached)?;
    load_runtime(cached.clone(), "managed DirectML cache").map_err(|error| {
        ToolError::new(
            TOOL,
            format!(
                "verified DirectML runtime was installed at {} but could not be loaded: {}\nPrevious attempts:\n  {}\nInstall the Microsoft Visual C++ 2015-2022 x64 Redistributable if its runtime DLLs are missing.",
                cached.display(),
                error.message(),
                errors.join("\n  ")
            ),
        )
    })
}

fn initialize_custom_provider_runtime(provider: Provider) -> ToolResult<RuntimeInfo> {
    let asset = current_asset()?;
    let executable = env::current_exe().ok();
    let candidates = runtime_candidates(asset, Path::new(""), executable.as_deref(), false);
    let mut errors = Vec::new();
    for candidate in candidates {
        match load_runtime(candidate.clone(), "application runtime") {
            Ok(runtime) => return Ok(runtime),
            Err(error) => errors.push(format!("{}: {}", candidate.display(), error.message())),
        }
    }
    Err(ToolError::new(
        TOOL,
        format!(
            "{} requires a provider-enabled ONNX Runtime. Set ORT_DYLIB_PATH to its absolute library path.\n{}\nAttempts:\n  {}",
            provider.name(),
            provider_setup(provider),
            errors.join("\n  ")
        ),
    ))
}

fn initialize_cpu_runtime(download_approved: bool) -> ToolResult<RuntimeInfo> {
    let asset = current_asset()?;
    let cached = cache_library(asset)?;
    let executable = env::current_exe().ok();
    let candidates = runtime_candidates(asset, &cached, executable.as_deref(), true);
    let mut errors = Vec::new();
    for candidate in &candidates {
        match load_runtime(candidate.clone(), candidate_source(candidate, &cached)) {
            Ok(runtime) => return Ok(runtime),
            Err(error) => errors.push(format!("{}: {}", candidate.display(), error.message())),
        }
    }

    if !download_approved {
        confirm_install(asset, &cached)?;
    }
    install_runtime(asset, &cached)?;
    load_runtime(cached.clone(), "managed CPU cache").map_err(|error| {
        ToolError::new(
            TOOL,
            format!(
                "verified ONNX Runtime was installed at {} but could not be loaded: {}\nPrevious attempts:\n  {}",
                cached.display(),
                error.message(),
                errors.join("\n  ")
            ),
        )
    })
}

fn candidate_source(candidate: &Path, cached: &Path) -> &'static str {
    if candidate == cached {
        "managed CPU cache"
    } else if candidate.is_absolute() {
        "application runtime"
    } else {
        "system loader"
    }
}

fn runtime_candidates(
    asset: RuntimeAsset,
    cached: &Path,
    executable: Option<&Path>,
    include_cache: bool,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(directory) = executable.and_then(Path::parent) {
        candidates.push(directory.join(asset.library_name));
        candidates.push(directory.join("lib").join(asset.library_name));
    }
    if include_cache {
        candidates.push(cached.to_path_buf());
    }

    // Never ask Windows to resolve a bare onnxruntime.dll: System32 contains
    // private copies that can poison ort's process-global runtime selection.
    #[cfg(not(target_os = "windows"))]
    candidates.push(PathBuf::from(asset.library_name));

    candidates
}

fn load_runtime(path: PathBuf, source: &'static str) -> ToolResult<RuntimeInfo> {
    let canonical = initialize_from(&path).map_err(|error| {
        ToolError::new(
            TOOL,
            format!("cannot load ONNX Runtime from {}: {error}", path.display()),
        )
    })?;
    Ok(RuntimeInfo {
        path: canonical,
        source,
        providers: Vec::new(),
        attempts: Vec::new(),
        auto_cpu_fallback: false,
    })
}

fn initialize_from(path: &Path) -> Result<PathBuf, String> {
    let load_path = path
        .canonicalize()
        .map_err(|error| format!("runtime file {} is unavailable: {error}", path.display()))?;
    if let Some(active) = INITIALIZED_RUNTIME.get() {
        return if active == &load_path {
            Ok(active.clone())
        } else {
            Err(format!(
                "ONNX Runtime is already initialized from {}; refusing conflicting runtime {}",
                active.display(),
                load_path.display()
            ))
        };
    }

    let committed = ort::init_from(&load_path)
        .map_err(|error| error.to_string())?
        .commit();
    if committed {
        let _ = INITIALIZED_RUNTIME.set(load_path.clone());
        return Ok(load_path);
    }

    if let Some(active) = INITIALIZED_RUNTIME.get() {
        if active == &load_path {
            Ok(active.clone())
        } else {
            Err(format!(
                "ONNX Runtime was initialized concurrently from {}; requested {}",
                active.display(),
                load_path.display()
            ))
        }
    } else {
        Err("ONNX Runtime was initialized before justrmbg could record its library path; run this check in a fresh process".to_owned())
    }
}

fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

fn confirm_dml_install(target: &Path) -> ToolResult {
    let mut input = io::stdin().lock();
    confirm_dml_install_with(target, is_interactive(), &mut input)
}

fn confirm_dml_install_with<R: BufRead>(
    target: &Path,
    interactive: bool,
    input: &mut R,
) -> ToolResult {
    if !interactive {
        return Err(ToolError::new(
            TOOL,
            format!(
                "managed DirectML was not found. Refusing to download without interactive confirmation.\nRun `justrmbg --check --gpu` in a terminal to approve the verified Microsoft packages, or set ORT_DYLIB_PATH to an absolute provider-enabled runtime.\nPackages:\n  {}\n  {}",
                DML_PACKAGES[0].url, DML_PACKAGES[1].url
            ),
        ));
    }
    let total = DML_PACKAGES
        .iter()
        .map(|package| package.bytes)
        .sum::<u64>();
    eprintln!(
        "DirectML acceleration is not installed.\n\
         Download: {} MiB from two official Microsoft NuGet packages\n\
         Packages: {} v{}\n\
                   {} v{}\n\
         SHA-256: {}\n\
                   {}\n\
         Target:   {}\n\
         DirectML supports compatible DirectX 12 GPUs from NVIDIA, AMD, and Intel.\n\
         Download now? [y/N]",
        total.div_ceil(1024 * 1024),
        DML_PACKAGES[0].name,
        DML_PACKAGES[0].version,
        DML_PACKAGES[1].name,
        DML_PACKAGES[1].version,
        DML_PACKAGES[0].sha256,
        DML_PACKAGES[1].sha256,
        target.display()
    );
    confirm_answer(
        input,
        "DirectML runtime download cancelled; no files were changed",
    )
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
                "ONNX Runtime was not found. Refusing to download without interactive confirmation.\nRun in a terminal to approve the verified official runtime, package {} beside the executable, or set ORT_DYLIB_PATH to an absolute path.\nOfficial asset: {}",
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
        asset.bytes.div_ceil(1024 * 1024),
        asset.version,
        asset.url(),
        asset.sha256,
        target.display()
    );
    confirm_answer(
        input,
        "ONNX Runtime download cancelled; no files were changed",
    )
}

fn confirm_answer<R: BufRead>(input: &mut R, declined: &str) -> ToolResult {
    let mut answer = String::new();
    input
        .read_line(&mut answer)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(ToolError::new(TOOL, declined))
    }
}

fn install_directml(library: &Path) -> ToolResult {
    let directory = library.parent().ok_or_else(|| {
        ToolError::new(TOOL, format!("invalid runtime path: {}", library.display()))
    })?;
    fs::create_dir_all(directory).map_err(|error| ToolError::new(TOOL, error.to_string()))?;

    let mut archives = Vec::new();
    for package in DML_PACKAGES {
        let mut archive = tempfile::Builder::new()
            .prefix("directml-")
            .suffix(".nupkg.partial")
            .tempfile_in(directory)
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        eprintln!("{TOOL}: downloading {} ...", package.url);
        download(
            package.url.to_owned(),
            package.bytes,
            package.sha256,
            &mut archive,
        )?;
        archives.push(archive);
    }

    for member in DML_MEMBERS.iter().filter(|member| !member.main) {
        extract_exact_zip_member(
            archives[member.package].path(),
            member,
            &directory.join(member.installed_name),
        )?;
    }
    write_dml_manifest(directory)?;
    let main = DML_MEMBERS.iter().find(|member| member.main).unwrap();
    extract_exact_zip_member(
        archives[main.package].path(),
        main,
        &directory.join(main.installed_name),
    )?;
    eprintln!(
        "{TOOL}: DirectML runtime installed at {}",
        library.display()
    );
    Ok(())
}

fn write_dml_manifest(directory: &Path) -> ToolResult {
    let manifest = RuntimeManifest {
        flavor: "directml-windows-x86_64".to_owned(),
        onnxruntime_version: DML_PACKAGES[0].version.to_owned(),
        directml_version: DML_PACKAGES[1].version.to_owned(),
        files: DML_MEMBERS
            .iter()
            .map(|member| ManifestFile {
                path: member.installed_name.to_owned(),
                bytes: member.bytes,
                sha256: member.sha256.to_owned(),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    write_atomic(
        &directory.join("manifest.json"),
        &mut bytes.as_slice(),
        None,
        None,
    )
}

fn validate_dml_cache(library: &Path) -> ToolResult<bool> {
    let expected = DML_MEMBERS
        .iter()
        .map(|member| ManifestFile {
            path: member.installed_name.to_owned(),
            bytes: member.bytes,
            sha256: member.sha256.to_owned(),
        })
        .collect::<Vec<_>>();
    validate_runtime_cache(
        library,
        "directml-windows-x86_64",
        DML_PACKAGES[0].version,
        DML_PACKAGES[1].version,
        &expected,
    )
}

fn validate_runtime_cache(
    library: &Path,
    flavor: &str,
    onnxruntime_version: &str,
    directml_version: &str,
    expected: &[ManifestFile],
) -> ToolResult<bool> {
    let Some(directory) = library.parent() else {
        return Ok(false);
    };
    let manifest_path = directory.join("manifest.json");
    let bytes = match fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ToolError::new(TOOL, error.to_string())),
    };
    let manifest: RuntimeManifest = match serde_json::from_slice(&bytes) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(false),
    };
    if manifest.flavor != flavor
        || manifest.onnxruntime_version != onnxruntime_version
        || manifest.directml_version != directml_version
        || manifest.files.len() != expected.len()
    {
        return Ok(false);
    }
    for expected_file in expected {
        let matching = manifest
            .files
            .iter()
            .filter(|file| file.path == expected_file.path)
            .collect::<Vec<_>>();
        if matching.len() != 1
            || matching[0].bytes != expected_file.bytes
            || matching[0].sha256 != expected_file.sha256
        {
            return Ok(false);
        }
        let path = directory.join(&expected_file.path);
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(false),
        };
        if !metadata.is_file() || metadata.len() != expected_file.bytes {
            return Ok(false);
        }
        if !expected_file.sha256.is_empty() && hash_file(&path)? != expected_file.sha256 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn hash_file(path: &Path) -> ToolResult<String> {
    let mut file = File::open(path).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn extract_exact_zip_member(
    archive_path: &Path,
    requested: &PackageMember,
    destination: &Path,
) -> ToolResult {
    let file = File::open(archive_path).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| ToolError::new(TOOL, format!("invalid runtime ZIP: {error}")))?;
    let matches = (0..archive.len())
        .filter(|index| {
            archive
                .by_index(*index)
                .ok()
                .is_some_and(|entry| entry.is_file() && entry.name() == requested.archive_path)
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(ToolError::new(
            TOOL,
            format!(
                "runtime archive contains {} copies of {}; expected exactly one",
                matches.len(),
                requested.archive_path
            ),
        ));
    }
    let mut entry = archive
        .by_index(matches[0])
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    if entry.size() != requested.bytes {
        return Err(ToolError::new(
            TOOL,
            format!(
                "{} is {} bytes; expected {}",
                requested.archive_path,
                entry.size(),
                requested.bytes
            ),
        ));
    }
    let hash = (!requested.sha256.is_empty()).then_some(requested.sha256);
    write_atomic(destination, &mut entry, Some(requested.bytes), hash)
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
    extract_member(asset, archive.path(), asset.library_name, library, true)?;
    eprintln!("{TOOL}: ONNX Runtime installed at {}", library.display());
    Ok(())
}

fn download_asset(asset: RuntimeAsset, output: &mut NamedTempFile) -> ToolResult {
    download(asset.url(), asset.bytes, asset.sha256, output)
}

fn download(url: String, bytes: u64, sha256: &str, output: &mut NamedTempFile) -> ToolResult {
    let mut response = ureq::get(&url)
        .call()
        .map_err(|error| ToolError::new(TOOL, format!("runtime download failed: {error}")))?;
    let announced = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if let Some(length) = announced
        && length != bytes
    {
        return Err(ToolError::new(
            TOOL,
            format!("runtime server announced {length} bytes; expected {bytes}"),
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
        if received > bytes {
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
    if received != bytes {
        return Err(ToolError::new(
            TOOL,
            format!("runtime download received {received} bytes; expected {bytes}"),
        ));
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != sha256 {
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
    let matches = (0..archive.len())
        .filter(|index| {
            archive.by_index(*index).ok().is_some_and(|entry| {
                entry.is_file() && is_archive_member(Path::new(entry.name()), requested, main)
            })
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(ToolError::new(
            TOOL,
            format!(
                "runtime archive contains {} matching {requested} files",
                matches.len()
            ),
        ));
    }
    let mut entry = archive
        .by_index(matches[0])
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    write_atomic(destination, &mut entry, None, None)
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
    let mut found = false;
    for entry in entries {
        let mut entry = entry.map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if is_archive_member(&path, requested, main) {
            if found {
                return Err(ToolError::new(
                    TOOL,
                    format!("runtime archive contains multiple matching {requested} files"),
                ));
            }
            write_atomic(destination, &mut entry, None, None)?;
            found = true;
        }
    }
    if found {
        Ok(())
    } else {
        Err(ToolError::new(
            TOOL,
            format!("runtime archive lacks {requested}"),
        ))
    }
}

fn write_atomic<R: Read>(
    destination: &Path,
    reader: &mut R,
    expected_bytes: Option<u64>,
    expected_sha256: Option<&str>,
) -> ToolResult {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    }
    let mut output = AtomicWriteFile::open(destination)
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut written = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if count == 0 {
            break;
        }
        written += count as u64;
        if expected_bytes.is_some_and(|expected| written > expected) {
            return Err(ToolError::new(
                TOOL,
                "runtime member exceeded its pinned size",
            ));
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        hasher.update(&buffer[..count]);
    }
    if expected_bytes.is_some_and(|expected| written != expected) {
        return Err(ToolError::new(
            TOOL,
            format!(
                "runtime member was {written} bytes; expected {}",
                expected_bytes.unwrap()
            ),
        ));
    }
    if let Some(expected) = expected_sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            return Err(ToolError::new(
                TOOL,
                format!("runtime member failed SHA-256 verification (got {actual})"),
            ));
        }
    }
    output
        .flush()
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    output
        .commit()
        .map_err(|error| ToolError::new(TOOL, format!("cannot install runtime: {error}")))
}

pub struct Engine {
    session: Option<Session>,
    provider: Provider,
    uses_cpu_support: bool,
}

impl Engine {
    pub fn create(
        model: &Path,
        requested: Provider,
        runtime: &RuntimeInfo,
    ) -> ToolResult<(Self, Vec<String>)> {
        let mut attempts = runtime.attempts.clone();
        for provider in runtime.providers.iter().copied() {
            if provider == Provider::Cpu {
                return Ok((Self::cpu(model)?, attempts));
            }
            let probe = Self::with_provider_memory(&probe_model(), provider, false)
                .and_then(|mut engine| engine.probe_infer());
            if let Err(error) = probe {
                if requested != Provider::Auto {
                    return Err(error);
                }
                attempts.push(format!(
                    "{} unavailable: {}; trying the next provider",
                    provider.name(),
                    error.message()
                ));
                continue;
            }
            let allow_cpu_support = requested == Provider::Auto;
            match Self::with_provider_file(model, provider, allow_cpu_support) {
                Ok(engine) => {
                    if engine.uses_cpu_support {
                        attempts.push(format!(
                            "Auto selected {} acceleration with CPU support for model nodes the provider cannot execute",
                            provider.name()
                        ));
                    }
                    return Ok((engine, attempts));
                }
                Err(error) if requested == Provider::Auto => attempts.push(format!(
                    "{} unavailable: {}; trying the next provider",
                    provider.name(),
                    error.message()
                )),
                Err(error) => return Err(error),
            }
        }
        if requested == Provider::Auto && !runtime.auto_cpu_fallback {
            attempts.push("Auto selected CPU".to_owned());
        }
        Ok((Self::cpu(model)?, attempts))
    }

    pub fn cpu(model: &Path) -> ToolResult<Self> {
        Self::with_provider_file(model, Provider::Cpu, false)
    }

    fn with_provider_file(
        model: &Path,
        provider: Provider,
        allow_cpu_support: bool,
    ) -> ToolResult<Self> {
        let mut builder = configure_provider(session_builder()?, provider, allow_cpu_support)?;
        let session = builder.commit_from_file(model).map_err(|error| {
            ToolError::new(
                TOOL,
                format!("cannot load model with {}: {error}", provider.name()),
            )
        })?;
        Ok(Self {
            session: Some(session),
            provider,
            uses_cpu_support: provider != Provider::Cpu && allow_cpu_support,
        })
    }

    fn with_provider_memory(
        model: &[u8],
        provider: Provider,
        allow_cpu_support: bool,
    ) -> ToolResult<Self> {
        let mut builder = configure_provider(session_builder()?, provider, allow_cpu_support)?;
        let session = builder.commit_from_memory(model).map_err(|error| {
            ToolError::new(
                TOOL,
                format!("cannot create {} probe session: {error}", provider.name()),
            )
        })?;
        Ok(Self {
            session: Some(session),
            provider,
            uses_cpu_support: provider != Provider::Cpu && allow_cpu_support,
        })
    }

    pub fn provider(&self) -> Provider {
        self.provider
    }

    pub fn is_gpu(&self) -> bool {
        self.provider != Provider::Cpu
    }

    pub fn replace_with_cpu(&mut self, model: &Path) -> ToolResult {
        self.session.take();
        let cpu = Self::cpu(model)?;
        *self = cpu;
        Ok(())
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
            .as_mut()
            .ok_or_else(|| ToolError::new(TOOL, "inference session is not initialized"))?
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

    fn probe_infer(&mut self) -> ToolResult {
        let left = Tensor::from_array(([4_usize], vec![1_f32, 2.0, 3.0, 4.0].into_boxed_slice()))
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        let right =
            Tensor::from_array(([4_usize], vec![10_f32, 20.0, 30.0, 40.0].into_boxed_slice()))
                .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        let outputs = self
            .session
            .as_mut()
            .ok_or_else(|| ToolError::new(TOOL, "probe session is not initialized"))?
            .run(ort::inputs![left, right])
            .map_err(|error| ToolError::new(TOOL, format!("probe inference failed: {error}")))?;
        let (_, output) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
        if output != [11_f32, 22.0, 33.0, 44.0] {
            return Err(ToolError::new(TOOL, "probe returned an unexpected value"));
        }
        Ok(())
    }
}

fn configure_provider(
    mut builder: ort::session::builder::SessionBuilder,
    provider: Provider,
    allow_cpu_support: bool,
) -> ToolResult<ort::session::builder::SessionBuilder> {
    let dispatch = match provider {
        Provider::Cpu => return Ok(builder),
        Provider::Cuda => ep::CUDA::default().build().error_on_failure(),
        Provider::DirectMl => ep::DirectML::default().build().error_on_failure(),
        Provider::CoreMl => ep::CoreML::default().build().error_on_failure(),
        Provider::Auto => return Err(ToolError::new(TOOL, "Auto is not an execution provider")),
    };
    if provider == Provider::DirectMl {
        builder = builder
            .with_memory_pattern(false)
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    }
    if !allow_cpu_support {
        builder = builder
            .with_disable_cpu_fallback()
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    }
    builder
        .with_execution_providers([dispatch])
        .map_err(|error| {
            ToolError::new(
                TOOL,
                format!("{} registration failed: {error}", provider.name()),
            )
        })
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

fn requested_providers(requested: Provider) -> ToolResult<Vec<Provider>> {
    if requested != Provider::Auto {
        return Ok(vec![requested]);
    }
    if let Ok(value) = env::var("RMBG_GPU_PROVIDERS") {
        let configured = value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                Provider::parse(value)
                    .filter(|provider| !matches!(provider, Provider::Auto | Provider::Cpu))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                ToolError::new(
                    TOOL,
                    "RMBG_GPU_PROVIDERS accepts only directml, dml, cuda, and coreml",
                )
            })?;
        if !configured.is_empty() {
            return Ok(configured);
        }
    }
    Ok(if cfg!(target_os = "windows") {
        vec![Provider::DirectMl, Provider::Cuda]
    } else if cfg!(target_os = "macos") {
        vec![Provider::CoreMl]
    } else {
        vec![Provider::Cuda]
    })
}

fn provider_setup(provider: Provider) -> &'static str {
    match provider {
        Provider::DirectMl => {
            "On Windows x64, run `justrmbg --check --gpu` interactively to install managed DirectML."
        }
        Provider::Cuda => {
            "Install an ONNX Runtime GPU build compatible with your CUDA and cuDNN versions, then set ORT_DYLIB_PATH."
        }
        Provider::CoreMl => {
            "Install a CoreML-enabled ONNX Runtime and set ORT_DYLIB_PATH to its library."
        }
        Provider::Cpu => "Use the managed CPU runtime or set ORT_DYLIB_PATH.",
        Provider::Auto => "Use `justrmbg --check` to inspect automatic selection.",
    }
}

pub fn check(requested: Provider, runtime: &RuntimeInfo) -> ToolResult {
    println!("Requested provider: {}", requested.name());
    println!("Runtime source: {}", runtime.source);
    println!("Runtime path: {}", runtime.path.display());

    let mut failures = runtime.attempts.clone();
    for provider in runtime.providers.iter().copied() {
        match Engine::with_provider_memory(&probe_model(), provider, false)
            .and_then(|mut engine| engine.probe_infer().map(|_| engine))
        {
            Ok(_) => {
                for failure in failures {
                    eprintln!("{TOOL}: {failure}");
                }
                if provider == Provider::Cpu
                    && (requested == Provider::Auto || runtime.auto_cpu_fallback)
                {
                    println!("Selected provider: CPU (Auto fallback)");
                } else {
                    println!("Selected provider: {}", provider.name());
                }
                println!("Check: OK (session creation and inference succeeded)");
                return Ok(());
            }
            Err(error) if requested == Provider::Auto => {
                failures.push(format!(
                    "{} unavailable: {}; trying the next provider",
                    provider.name(),
                    error.message()
                ));
                continue;
            }
            Err(error) => {
                return Err(ToolError::new(
                    TOOL,
                    format!(
                        "{} check failed: {}\n{}",
                        provider.name(),
                        error.message(),
                        provider_setup(provider)
                    ),
                ));
            }
        }
    }

    if requested != Provider::Auto {
        return Err(ToolError::new(TOOL, "requested provider was not available"));
    }
    let mut engine = Engine::with_provider_memory(&probe_model(), Provider::Cpu, false)?;
    engine.probe_infer()?;
    for failure in failures {
        eprintln!("{TOOL}: {failure}");
    }
    println!("Selected provider: CPU (Auto fallback)");
    println!("Check: OK (session creation and inference succeeded)");
    Ok(())
}

fn probe_model() -> Vec<u8> {
    fn varint(mut value: u64, output: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                break;
            }
        }
    }
    fn field_varint(field: u8, value: u64, output: &mut Vec<u8>) {
        output.push(field << 3);
        varint(value, output);
    }
    fn field_bytes(field: u8, value: &[u8], output: &mut Vec<u8>) {
        output.push((field << 3) | 2);
        varint(value.len() as u64, output);
        output.extend_from_slice(value);
    }
    fn dimension() -> Vec<u8> {
        let mut value = Vec::new();
        field_varint(1, 4, &mut value);
        value
    }
    fn value_info(name: &str) -> Vec<u8> {
        let mut shape = Vec::new();
        field_bytes(1, &dimension(), &mut shape);
        let mut tensor = Vec::new();
        field_varint(1, 1, &mut tensor);
        field_bytes(2, &shape, &mut tensor);
        let mut r#type = Vec::new();
        field_bytes(1, &tensor, &mut r#type);
        let mut info = Vec::new();
        field_bytes(1, name.as_bytes(), &mut info);
        field_bytes(2, &r#type, &mut info);
        info
    }

    let mut node = Vec::new();
    field_bytes(1, b"left", &mut node);
    field_bytes(1, b"right", &mut node);
    field_bytes(2, b"output", &mut node);
    field_bytes(4, b"Add", &mut node);

    let mut graph = Vec::new();
    field_bytes(1, &node, &mut graph);
    field_bytes(2, b"justrmbg-provider-probe", &mut graph);
    field_bytes(11, &value_info("left"), &mut graph);
    field_bytes(11, &value_info("right"), &mut graph);
    field_bytes(12, &value_info("output"), &mut graph);

    let mut opset = Vec::new();
    field_varint(2, 13, &mut opset);

    let mut model = Vec::new();
    field_varint(1, 8, &mut model);
    field_bytes(2, b"JustTools", &mut model);
    field_bytes(7, &graph, &mut model);
    field_bytes(8, &opset, &mut model);
    model
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
        }
        assert!(asset_for("windows", "x86").is_none());
        assert!(asset_for("freebsd", "x86_64").is_none());
    }

    #[test]
    fn directml_packages_and_binary_members_are_pinned() {
        for package in DML_PACKAGES {
            assert!(package.url.starts_with("https://api.nuget.org/"));
            assert_eq!(package.sha256.len(), 64);
            assert!(package.bytes > 0);
        }
        for member in DML_MEMBERS {
            assert!(member.bytes > 0);
            if member.installed_name.ends_with(".dll") {
                assert_eq!(member.sha256.len(), 64);
            }
        }
        assert_eq!(DML_MEMBERS.iter().filter(|member| member.main).count(), 1);
    }

    fn cache_fixture(directory: &Path) -> (PathBuf, Vec<ManifestFile>) {
        let files = [
            ("onnxruntime.dll", b"runtime".as_slice()),
            ("DirectML.dll", b"directml".as_slice()),
            ("notices/LICENSE.txt", b"license".as_slice()),
        ];
        let expected = files
            .iter()
            .map(|(path, bytes)| {
                let target = directory.join(path);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(&target, bytes).unwrap();
                ManifestFile {
                    path: (*path).to_owned(),
                    bytes: bytes.len() as u64,
                    sha256: hash_file(&target).unwrap(),
                }
            })
            .collect::<Vec<_>>();
        let manifest = RuntimeManifest {
            flavor: "test-directml".to_owned(),
            onnxruntime_version: "1.0".to_owned(),
            directml_version: "2.0".to_owned(),
            files: expected
                .iter()
                .map(|file| ManifestFile {
                    path: file.path.clone(),
                    bytes: file.bytes,
                    sha256: file.sha256.clone(),
                })
                .collect(),
        };
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        (directory.join("onnxruntime.dll"), expected)
    }

    #[test]
    fn runtime_cache_validation_accepts_exact_fixture() {
        let directory = tempfile::tempdir().unwrap();
        let (library, expected) = cache_fixture(directory.path());

        assert!(
            validate_runtime_cache(&library, "test-directml", "1.0", "2.0", &expected).unwrap()
        );
    }

    #[test]
    fn runtime_cache_validation_rejects_tampered_member() {
        let directory = tempfile::tempdir().unwrap();
        let (library, expected) = cache_fixture(directory.path());
        fs::write(directory.path().join("DirectML.dll"), b"tampered").unwrap();

        assert!(
            !validate_runtime_cache(&library, "test-directml", "1.0", "2.0", &expected).unwrap()
        );
    }

    #[test]
    fn runtime_cache_validation_rejects_missing_member() {
        let directory = tempfile::tempdir().unwrap();
        let (library, expected) = cache_fixture(directory.path());
        fs::remove_file(directory.path().join("notices/LICENSE.txt")).unwrap();

        assert!(
            !validate_runtime_cache(&library, "test-directml", "1.0", "2.0", &expected).unwrap()
        );
    }

    #[test]
    fn runtime_cache_validation_rejects_duplicate_manifest_record() {
        let directory = tempfile::tempdir().unwrap();
        let (library, expected) = cache_fixture(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: RuntimeManifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest.files.pop();
        manifest.files.push(ManifestFile {
            path: manifest.files[0].path.clone(),
            bytes: manifest.files[0].bytes,
            sha256: manifest.files[0].sha256.clone(),
        });
        fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

        assert!(
            !validate_runtime_cache(&library, "test-directml", "1.0", "2.0", &expected).unwrap()
        );
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
    fn noninteractive_directml_install_makes_no_directories() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("new/cache/onnxruntime.dll");
        let mut input = Cursor::new(b"yes\n");
        let error = confirm_dml_install_with(&target, false, &mut input).unwrap_err();
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

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_runtime_candidates_never_use_ambiguous_dll_search() {
        let asset = asset_for("windows", "x86_64").unwrap();
        let executable = Path::new(r"C:\cmd\bin\justrmbg.exe");
        let cached = Path::new(
            r"C:\Users\tester\AppData\Local\JustGains\JustTools\cache\onnxruntime\cpu-1.24.3-windows-x86_64\onnxruntime.dll",
        );
        let candidates = runtime_candidates(asset, cached, Some(executable), true);
        assert_eq!(
            candidates,
            vec![
                PathBuf::from(r"C:\cmd\bin\onnxruntime.dll"),
                PathBuf::from(r"C:\cmd\bin\lib\onnxruntime.dll"),
                cached.to_path_buf(),
            ]
        );
        assert!(candidates.iter().all(|candidate| candidate.is_absolute()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_missing_runtime_is_rejected_before_dll_loading() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("onnxruntime.dll");
        let error = initialize_from(&missing).unwrap_err();
        assert!(error.contains("runtime file"));
        assert!(error.contains("is unavailable"));
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

    #[test]
    fn probe_model_has_stable_nonempty_bytes() {
        let first = probe_model();
        assert_eq!(first, probe_model());
        assert!(first.len() < 256);
    }
}
