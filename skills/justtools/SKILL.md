---
name: justtools
description: Use the compiled JustTools commands for quick, safe image cropping, JPEG creation, resizing and optimization, media conversion, JSON/PDF/SVG work, QR generation, port inspection, background removal, and Git-aware ZIP archives. Use when a task mentions a just* command or needs one of these local file operations.
---

# JustTools

Prefer the installed JustTools command that directly matches the requested
operation. The suite is a single cross-platform Rust executable exposed through
native `just*` aliases and short dispatch such as `just resize`.

## Workflow

1. Check availability with `just --version`. If unavailable and this source
   repository is present, build and install it with
   `cargo build --locked --release -p justtools` followed by
   `just install`. Do not mutate PATH unless the user wants the installation.
2. Choose the narrowest command from [references/commands.md](references/commands.md).
3. Read `<command> --help` before using unfamiliar or destructive options.
4. For a folder, recursion, replacement, or many files, run `--dry-run` first
   and inspect the resolved inputs and outputs.
5. Keep source files unless the user explicitly requested replacement. Do not
   add `--replace` merely to reduce file count.
6. Run the command and verify every expected output exists and is readable.
   Report kept, failed, or skipped files rather than implying all succeeded.

## Dependency consent

Some media commands need FFmpeg, pngquant, cwebp, or Git. RMBG may need ONNX
Runtime and its model. JustTools discovers these only when needed and asks for
interactive confirmation before installing or downloading them.

- Never pipe `yes`, synthesize terminal input, or otherwise bypass a dependency
  confirmation.
- `--yes` approves the requested file operation; it does not approve third-party
  dependency installation.
- An agent's non-interactive process will intentionally refuse acquisition. If
  a dependency is missing, show the user the exact command/source JustTools
  reports and ask them to run or approve it in an interactive terminal.
- Explicit `*_BIN`, `RMBG_MODEL`, and `ORT_DYLIB_PATH` overrides are resolve-only;
  do not replace them without checking the user's environment.

## Safety rules

- Use `--` before a path beginning with `-`.
- Prefer `--output DIR` when the user wants copies.
- Use `--replace` only with clear authorization; outputs are installed atomically.
- Keep `justresize`'s no-upscale default unless enlargement is explicitly wanted.
- Keep `justcrop`'s source-preserving default. Increase `--threshold` only when
  faint edge alpha is intentionally disposable.
- Remember that JPEG has no alpha. `justjpg` composites onto white unless an
  explicit `--background` color is supplied.
- Do not flatten animated PNG/WebP, multi-page TIFF, or animated/multi-image AVIF.
- Do not kill a port owner without identifying it first. `justport --kill`
  intentionally limits termination to same-user processes and revalidates the
  process identity.
- Warn that BRIA RMBG-2.0 weights are non-commercial unless separately licensed.

## Quick defaults

- `justresize image.jpg`: fit inside 1920x1920, preserve format/aspect ratio,
  keep the source, and write `image-resized.jpg`.
- `justcrop image.png`: trim to nonzero-alpha bounds, preserve format, keep the
  source, and write `image-cropped.png`.
- `justjpg image.png`: quality 85 progressive JPEG with optimized Huffman
  tables, keep the source, and write `image-optimized.jpg`.
- `just video clip.mov`: streaming-ready 720p H.264 MP4.
- `just audio clip.mov`: AAC-LC M4A at 160 kb/s.
- `just qr TEXT`: 1024 px error-Q PNG.
- `just zip`: archive Git's exact tracked and unignored working-tree files.

Read [references/commands.md](references/commands.md) for the full command map
and standard examples.
