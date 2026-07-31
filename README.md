# JustTools

Fast, opinionated, cross-platform `just*` commands in one compiled Rust
executable. There is no Node, Bun, or PowerShell runtime layer. `just install`
creates native aliases, so the same binary runs as `justjson`, `justpdf`,
`justvideo`, and every other command.

## Install

Build from source, then let the binary install itself:

```powershell
cargo build --locked --release -p justtools
.\target\release\just.exe install
```

```sh
cargo build --locked --release -p justtools
./target/release/just install
```

The default destination is `C:\cmd\bin` when that directory already exists on
Windows, otherwise the per-user application directory on Windows and
`~/.local/bin` on macOS/Linux. Override it with `--bin-dir` or
`JUSTTOOLS_BIN`. Existing managed JustTools files are moved to a timestamped
recovery directory before the staged native install commits. A failed upgrade
rolls the prior installation back.

If the install directory is not on `PATH`, the `just` browser offers
**Add To Path**. The direct commands are `just add-to-path` and
`just install`. Open a new terminal after changing `PATH`.

## Commands and defaults

| Command | Opinionated default |
| --- | --- |
| `just` | Browse or directly dispatch every command; offer Add To Path when needed |
| `justaudio` | AAC-LC M4A, 160 kb/s, 48 kHz; keep the source |
| `justavif` | AV1 still image, quality 60, speed 6; install only when smaller |
| `justcrop` | Trim transparent borders to nonzero-alpha bounds; preserve format and source |
| `justjpg` | Quality 85 progressive 4:2:0 JPEG, optimized Huffman tables, white alpha background |
| `justjson` | Pretty-print in place with two spaces and a final newline |
| `justmp3` | LAME VBR quality 2, 48 kHz; keep the source |
| `justpdf` | Show info for one PDF or merge several into `merged.pdf` |
| `justpng` | pngquant quality 65-90, speed 3; replace only when smaller |
| `justport` | Show the owner of exact local ports; guarded same-user kill is optional |
| `justqr` | 1024 px, error-Q, four-module margin, black-on-white `qr.png` |
| `justresize` | Fit still images within 1920x1920, never upscale, preserve format and source |
| `justrmbg` / `rmbg` | Local BRIA RMBG-2.0 removal to `<name>-nobg.png` |
| `justsvg` | Conservative multipass SVGOMG-style optimization, precision 3 |
| `justvideo` | 720p H.264 MP4, CRF 28, medium preset, AAC 128 kb/s |
| `justwav` | Stereo 16-bit PCM WAV at 48 kHz; keep the source |
| `justwebp` | Quality 82, method 5 WebP; replace only when smaller |
| `justzip` | Smallest-compression ZIP from Git's exact tracked/unignored file set |

Run `just`, `just --help`, or `just help qr`. Short dispatch stays quick:

```sh
just qr "https://example.com"
just crop transparent-logo.png
just jpg photo.png
just resize photo.jpg --width 1200
just json package.json
just pdf report-a.pdf report-b.pdf
just mp3 interview.mov
just port 4321
just rmbg portrait.jpg
```

## Consistent file handling

- File tools accept files, folders, Unicode paths, and `--` before unusual path
  names. Folder scans are direct unless `--recursive` is selected.
- Batch tools consistently expose `--output`, `--yes`, `--dry-run`, and
  `--help`; encoders also expose `--jobs`.
- Inputs are normalized and deduplicated. Output collisions and input/output
  overlap are rejected before processing.
- Outputs are written beside their destination and atomically installed.
  Existing modes are preserved for in-place document edits.
- Destructive folder operations ask once. Explicit-file operations remain
  low-friction, while `--yes` makes automation intentional.
- Audio tools keep sources unless `--replace` is explicit. WebP and AVIF remove
  a beside-source original only after a smaller replacement is complete.
- APNG, animated WebP/AVIF, multi-page TIFF, and AVIF transparency are rejected
  where a still-image conversion would silently discard content.
- Crop applies EXIF orientation, trims to alpha values above the selected
  threshold, clamps optional padding to the original canvas, and reduces fully
  transparent canvases to a valid 1x1 transparent image.
- JPG applies EXIF orientation, composites transparency onto white by default,
  writes progressive 4:2:0 output with optimized Huffman tables, and strips
  metadata. Quality 90+ switches to full 4:4:4 chroma. Use `--background`,
  `--quality`, or `--baseline` when needed.
- Resize applies EXIF orientation, uses Lanczos3, strips metadata, and preserves
  aspect ratio unless exact center-cropping is explicitly selected.
- PDF inputs are never deleted. Page ranges are one-based, such as
  `1-3,5,last`.
- ZIP paths, symlinks, output self-inclusion, Git submodules, and ZIP64 files
  are handled explicitly; unsafe links that escape the archive root fail closed.

`justjson` and `justsvg` treat piped input as document content. `justqr` treats
piped input as text. Media, PDF, and RMBG commands accept paths; see each
command's `--help` for its exact batch behavior.

`justsvg` uses OXVG, a Rust-native optimizer modeled on SVGO/SVGOMG. Its
conservative preset preserves IDs, `viewBox`, titles, descriptions, XML
namespaces, and accessibility attributes.

Third-party attribution for the native JPEG encoder ships in
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) and every release archive.

## Dependencies: discover, explain, confirm

Pure-Rust crop, JPG, JSON, PDF, QR, resize, SVG, and port operations have no
external runtime dependencies. Other commands resolve dependencies only when
invoked:

| Commands | Dependency | Confirmed acquisition |
| --- | --- | --- |
| audio, AVIF, MP3, video, WAV | FFmpeg + ffprobe | WinGet, Homebrew, apt, dnf, or pacman |
| PNG | pngquant | package manager, or checksum-pinned official Windows archive |
| WebP | `cwebp` | WinGet or the platform WebP package |
| ZIP | Git | native package manager |
| RMBG | ONNX Runtime + BRIA model | checksum-pinned official runtime archive, then checksum-pinned S3 model archive |

When something is missing, JustTools prints the exact source and command, then
asks `[y/N]`. It runs the installer only after a real `y`/`yes` from an
interactive terminal. Redirected input, CI, and other non-interactive runs
never install or download anything; they return actionable instructions
instead. Declining leaves the filesystem unchanged.

Dependency installation cannot be bypassed with `--yes`. That flag applies to
the requested file operation or JustTools' own managed install, not third-party
software. Override discovery without enabling installation by setting
`FFMPEG_BIN`, `FFPROBE_BIN`, `PNGQUANT_BIN`, `CWEBP_BIN`, or `GIT_BIN` to a
specific executable.

The Fedora `ffmpeg-free` package can omit `libx264`, `libmp3lame`, or
`libaom-av1`. JustTools warns before installation and verifies the exact encoder
needed by the current command before touching inputs.

## RMBG runtime and model

`justrmbg` resolves ONNX Runtime before it offers the much larger model
download. A missing portable CPU runtime is fetched from Microsoft's official
release assets only after confirmation, with platform, size, and SHA-256 pinned
for Windows, Linux, and macOS on x64 and ARM64. `ORT_DYLIB_PATH` can point to a
provider-enabled runtime for DirectML, CUDA, or CoreML.

The BRIA RMBG-2.0 archive is hosted at
`https://m.justgains.com/tools/rmbg-2.0.zip`, verified by SHA-256, selectively
extracted, and installed atomically in the per-user cache. Use `--model` or
`RMBG_MODEL` for an existing ONNX file. Custom mirrors require both
`RMBG_MODEL_ARCHIVE_SHA256` and `RMBG_MODEL_SHA256` alongside
`RMBG_MODEL_URL`.

Automatic mode tries the platform GPU providers and falls back to CPU, including
an inference-time retry if a GPU provider initializes but fails. `--cpu` and
`--gpu` make that choice explicit.

BRIA publishes the RMBG-2.0 weights for non-commercial use. Commercial use
requires a separate agreement; review the
[BRIA model card and license](https://huggingface.co/briaai/RMBG-2.0) before
using the downloaded weights.

## Agent skill

The repository includes an installable `justtools` Agent Skill at
`skills/justtools`. The [`skills` npm CLI](https://www.npmjs.com/package/skills)
discovers it directly from GitHub:

```sh
npx skills add JustGains/JustTools --skill justtools
```

Target specific agents or install globally when desired:

```sh
npx skills add JustGains/JustTools --skill justtools --agent codex --agent claude-code
npx skills add JustGains/JustTools --skill justtools --agent codex --global
```

Use `npx skills add . --skill justtools` from a local clone while developing the
skill. Set `DISABLE_TELEMETRY=1` if you do not want the installer's anonymous
usage telemetry.

## Development and releases

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --release -p justtools
```

The test suite exercises every advertised command through the root binary,
native alias installation and rollback, non-interactive dependency refusal,
real JSON/PDF/QR/SVG/port behavior, Git-accurate ZIP creation, media safety,
native crop/JPG/resize behavior, and offline RMBG acquisition logic. CI also
checks the Rust 1.90 minimum and builds
native release archives for Windows x64/ARM64, Linux x64/ARM64 on an Ubuntu
22.04 compatibility baseline, and macOS Intel/Apple Silicon. Unix artifacts are
tarred so executable permissions survive artifact transport. Workflow actions
use their current Node 24-based major releases.

Every push and pull request keeps its native archives on the Actions run. A
version tag matching `Cargo.toml`, such as `v2.2.0`, additionally creates a
GitHub Release and attaches all six archives. Re-running the release job safely
replaces its existing assets.
