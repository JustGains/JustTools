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

/// A model file justrmbg can run. `Official` is BRIA's own ONNX export (the
/// managed download); the others are made from the same weights for one kind
/// of device and live beside it (see docs/rmbg.md, "Model files"):
///
/// - `OfficialHalf`: BRIA's export in half precision.
/// - `Fast`: a fixed 1024 x 1024 export whose deformable convolutions sample
///   every kernel point in one bilinear resample (GridSample) instead of
///   BRIA's gathers. TensorRT builds its half-precision engine from it.
/// - `FastHalf`: `Fast` in half precision, except where the deformable
///   convolutions compute and take their samples (float32). The GPU default.
/// - `Cpu`: the same convolutions one kernel point at a time, which keeps
///   memory low; twice as fast as BRIA's export on a CPU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Variant {
    Official,
    OfficialHalf,
    Fast,
    FastHalf,
    Cpu,
}

struct VariantFile {
    variant: Variant,
    name: &'static str,
    sha256: &'static str,
    max_bytes: u64,
}

/// Made with docs/rmbg.md's exact versions; a copy with another hash is skipped.
const VARIANT_FILES: [VariantFile; 4] = [
    VariantFile {
        variant: Variant::OfficialHalf,
        name: HALF_MODEL_NAME,
        sha256: HALF_MODEL_SHA256,
        max_bytes: MAX_HALF_MODEL_BYTES,
    },
    VariantFile {
        variant: Variant::Fast,
        name: "rmbg-2.0-fast.onnx",
        sha256: "d779938f65415623c01adff5016f1ab34cde6855d750b5a11610cd7f934536b2",
        max_bytes: 1_100_000_000,
    },
    VariantFile {
        variant: Variant::FastHalf,
        name: "rmbg-2.0-fast-fp16.onnx",
        sha256: "28454a831e24573b60fbe2e135163a64d132f9b40a3405c550b014c661cd370d",
        max_bytes: 600_000_000,
    },
    VariantFile {
        variant: Variant::Cpu,
        name: "rmbg-2.0-cpu.onnx",
        sha256: "8ab5fa6664ca12cc8592ea4ed9a7af48d4f6782ab239b1776a69793cd40e27fb",
        max_bytes: 1_100_000_000,
    },
];

impl Variant {
    /// The file's name without `.onnx`, as status lines and serve mode report it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Official => "rmbg-2.0",
            Self::OfficialHalf => "rmbg-2.0-fp16",
            Self::Fast => "rmbg-2.0-fast",
            Self::FastHalf => "rmbg-2.0-fast-fp16",
            Self::Cpu => "rmbg-2.0-cpu",
        }
    }

    /// The model files to try on `provider`, best first. Measured on an RTX
    /// 4090 and an i9-13900K (docs/rmbg.md): `FastHalf` takes 76 ms on DirectML
    /// against 156 for `OfficialHalf`; TensorRT runs `Fast` in 31 ms; `Cpu`
    /// takes 7.5 s against 14 for BRIA's export. `full` keeps float32 on GPUs
    /// (masks exactly like PyTorch's, about twice as slow).
    pub fn preferred(provider: Provider, full: bool) -> &'static [Variant] {
        use Variant::*;
        match provider {
            Provider::Cpu => &[Cpu, Official],
            Provider::TensorRt => &[Fast, Official],
            Provider::DirectMl | Provider::Cuda if full => &[Fast, Official],
            Provider::DirectMl | Provider::Cuda => &[FastHalf, OfficialHalf, Fast, Official],
            // CoreML has not been measured with the new files: as before.
            Provider::CoreMl if full => &[Official],
            Provider::CoreMl => &[OfficialHalf, Official],
            Provider::Auto => &[Official],
        }
    }
}

/// One model file to try.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelFile {
    pub path: PathBuf,
    /// `None` for an explicit model (`--model`, `RMBG_MODEL`).
    pub variant: Option<Variant>,
}

impl ModelFile {
    /// Checks a file beside the managed model against its pinned SHA-256 (the
    /// managed model itself and explicit models were checked when resolved).
    pub fn verify(&self) -> ToolResult {
        let Some(file) = self
            .variant
            .and_then(|variant| VARIANT_FILES.iter().find(|file| file.variant == variant))
        else {
            return Ok(());
        };
        verify_file(&self.path, file.max_bytes, file.sha256)
    }

    pub fn name(&self) -> String {
        match self.variant {
            Some(variant) => variant.name().to_owned(),
            None => self.path.file_stem().map_or_else(
                || "model".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            ),
        }
    }
}

/// The model files a run may use: an explicit model alone, or the managed
/// model and the verified files beside it.
#[derive(Clone, Debug)]
pub struct ModelSet {
    official: PathBuf,
    explicit: bool,
    full_precision: bool,
}

impl ModelSet {
    pub fn new(model: &Path, explicit: Option<&Path>, full_precision: bool) -> Self {
        let explicit =
            explicit.is_some() || env::var_os("RMBG_MODEL").is_some_and(|value| !value.is_empty());
        Self {
            official: model.to_path_buf(),
            explicit,
            full_precision,
        }
    }

    /// The files to try on `provider`, best first: those present beside the
    /// managed model, which ends every list. Check each with `verify` before
    /// loading it, so only the file actually used is hashed.
    pub fn for_provider(&self, provider: Provider) -> Vec<ModelFile> {
        if self.explicit {
            return vec![ModelFile {
                path: self.official.clone(),
                variant: None,
            }];
        }
        Variant::preferred(provider, self.full_precision)
            .iter()
            .filter_map(|&variant| {
                if variant == Variant::Official {
                    return Some(ModelFile {
                        path: self.official.clone(),
                        variant: Some(variant),
                    });
                }
                let file = VARIANT_FILES.iter().find(|file| file.variant == variant)?;
                let path = self.official.with_file_name(file.name);
                path.is_file().then_some(ModelFile {
                    path,
                    variant: Some(variant),
                })
            })
            .collect()
    }

    /// Every model file beside the managed model, so outputs never overwrite one.
    pub fn protected(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.official.clone()];
        if !self.explicit {
            paths.extend(
                VARIANT_FILES
                    .iter()
                    .map(|file| self.official.with_file_name(file.name)),
            );
        }
        paths
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
    // A file verified before and unchanged since (same size and modification
    // time) is not hashed again: that took about a second per gigabyte on every
    // start.
    let stamp = verification_stamp(&metadata, expected);
    let record = verification_record(path);
    if stamp.is_some() && fs::read_to_string(&record).ok() == stamp {
        return Ok(());
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
    if let Some(stamp) = stamp {
        // Best effort: without the record the next start hashes again.
        let _ = fs::write(&record, stamp);
    }
    Ok(())
}

/// Where a model's verification is remembered: `<file>.verified` beside it.
fn verification_record(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".verified");
    path.with_file_name(name)
}

/// What a verified file looked like: its expected hash, size and modification
/// time, or `None` when the system cannot say when it was modified.
fn verification_stamp(metadata: &fs::Metadata, expected: &str) -> Option<String> {
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{expected} {} {modified}\n", metadata.len()))
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
    fn each_provider_tries_its_best_files_first_and_the_managed_model_last() {
        use Variant::*;
        assert_eq!(Variant::preferred(Provider::Cpu, false), [Cpu, Official]);
        assert_eq!(
            Variant::preferred(Provider::TensorRt, false),
            [Fast, Official]
        );
        for gpu in [Provider::DirectMl, Provider::Cuda] {
            assert_eq!(
                Variant::preferred(gpu, false),
                [FastHalf, OfficialHalf, Fast, Official]
            );
            assert_eq!(Variant::preferred(gpu, true), [Fast, Official]);
        }
        for provider in [
            Provider::Cpu,
            Provider::TensorRt,
            Provider::DirectMl,
            Provider::Cuda,
            Provider::CoreMl,
            Provider::Auto,
        ] {
            for full in [false, true] {
                assert_eq!(Variant::preferred(provider, full).last(), Some(&Official));
            }
        }
    }

    #[test]
    fn files_beside_the_managed_model_are_offered_and_used_only_when_verified() {
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join(MODEL_NAME);
        fs::write(&model, b"full").unwrap();
        let official = ModelFile {
            path: model.clone(),
            variant: Some(Variant::Official),
        };
        let set = ModelSet::new(&model, None, false);
        assert_eq!(
            set.for_provider(Provider::DirectMl),
            std::slice::from_ref(&official)
        );
        official.verify().unwrap();
        for file in &VARIANT_FILES {
            fs::write(model.with_file_name(file.name), b"not the pinned model").unwrap();
        }
        let offered = set.for_provider(Provider::DirectMl);
        let names: Vec<String> = offered.iter().map(ModelFile::name).collect();
        assert_eq!(
            names,
            [
                "rmbg-2.0-fast-fp16",
                "rmbg-2.0-fp16",
                "rmbg-2.0-fast",
                "rmbg-2.0"
            ]
        );
        for file in &offered[..3] {
            assert!(file.verify().is_err(), "{}", file.name());
        }
        assert_eq!(set.for_provider(Provider::Cpu).len(), 2);
        assert_eq!(set.protected().len(), 1 + VARIANT_FILES.len());

        let explicit = ModelSet::new(&model, Some(&model), false);
        let only = explicit.for_provider(Provider::DirectMl);
        assert_eq!(only.len(), 1);
        assert_eq!(
            (only[0].variant, only[0].name()),
            (None, "rmbg-2.0".to_owned())
        );
        only[0].verify().unwrap();
        assert_eq!(explicit.protected(), [model]);
    }

    #[test]
    fn a_verified_file_is_not_hashed_again_until_it_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.onnx");
        let bytes = b"verified once";
        let hash = format!("{:x}", Sha256::digest(bytes));
        fs::write(&path, bytes).unwrap();
        verify_file(&path, 100, &hash).unwrap();
        let record = verification_record(&path);
        assert!(record.is_file());
        // the record vouches for the file as it was, under this hash only
        verify_file(&path, 100, &hash).unwrap();
        assert!(verify_file(&path, 100, MODEL_SHA256).is_err());
        fs::write(&path, b"changed, and longer than before").unwrap();
        assert!(verify_file(&path, 100, &hash).is_err());
    }

    #[test]
    fn a_file_is_verified_against_its_size_limit_and_hash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("half.onnx");
        let bytes = b"half precision test model";
        let hash = format!("{:x}", Sha256::digest(bytes));
        assert!(verify_file(&path, 100, &hash).is_err());
        fs::write(&path, bytes).unwrap();
        verify_file(&path, 100, &hash).unwrap();
        assert!(verify_file(&path, 100, MODEL_SHA256).is_err());
        assert!(verify_file(&path, 10, &hash).is_err());
    }

    #[test]
    fn pinned_hashes_are_well_formed() {
        let variants = VARIANT_FILES.iter().map(|file| file.sha256);
        for hash in [ARCHIVE_SHA256, MODEL_SHA256].into_iter().chain(variants) {
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
