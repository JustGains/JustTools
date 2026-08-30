---
name: justtools
description: Use the compiled JustTools commands for curated software setup, AI-assisted Git commits, process cleanup, quick and safe image work, media conversion, JSON/PDF/SVG operations, QR generation, port inspection, background removal, and Git-aware ZIP archives. Use when a task mentions JustReady, JustCommit, bunt, a just* command, or needs one of these local operations.
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

`justready` is intentionally a system-software installer. Use it only when the
user asked to prepare the machine or install apps. Start with `justready --list`
or an explicit `--dry-run`, show the resolved plan, and do not add `--yes`
without clear authorization to execute that plan. It may legitimately invoke
WinGet, Homebrew, a Linux package manager, Flatpak, or a fixed official
installer and may surface UAC or `sudo` prompts.

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
  faint edge alpha is intentionally disposable. Use `--shared-bounds` for frame
  folders so every image keeps the same dimensions and relative positioning.
- Remember that JPEG has no alpha. `justjpg` composites onto white unless an
  explicit `--background` color is supplied.
- Do not flatten animated PNG/WebP, multi-page TIFF, or animated/multi-image AVIF.
- Do not kill a port owner without identifying it first. `justport --kill`
  intentionally limits termination to same-user processes and revalidates the
  process identity.
- In `bunt`, protect workloads that must survive with `e` before using `K`.
  Bunt revalidates PID, start time, runtime, workload, and exclusions before
  terminating the captured target set.
- `justcommit` commits only the staged index unless `--all` is explicit. Prefer
  `--dry-run` when the user asked only for a proposed message. Never expose or
  pass an OpenRouter key to a repair agent.
- Warn that BRIA RMBG-2.0 weights are non-commercial unless separately licensed.

## Quick defaults

- `justresize image.jpg`: fit inside 1920x1920, preserve format/aspect ratio,
  keep the source, and write `image-resized.jpg`.
- `justcrop image.png`: trim to nonzero-alpha bounds, preserve format, keep the
  source, and write `image-cropped.png`.
- `justcrop frames --shared-bounds`: union visible bounds within each folder so
  animation frames remain aligned.
- `justjpg image.png`: quality 85 progressive JPEG with optimized Huffman
  tables, keep the source, and write `image-optimized.jpg`.
- `just video clip.mov`: streaming-ready 720p H.264 MP4.
- `just audio clip.mov`: AAC-LC M4A at 160 kb/s.
- `just qr TEXT`: 1024 px error-Q PNG.
- `bunt`: interactively inspect, protect, filter, and stop Node/Bun/Python
  processes; `bunt --snapshot` is read-only.
- `justready`: open the OS-filtered software picker; use `justready --list` for
  read-only inventory and `--install IDS --dry-run` to review exact commands.
- `justcommit`: summarize the staged index with bounded model input and create
  the commit; use `--dry-run` to print only and `--all` only with authorization
  to stage the complete worktree.
- `just zip`: archive Git's exact tracked and unignored working-tree files.

Read [references/commands.md](references/commands.md) for the full command map
and standard examples.
