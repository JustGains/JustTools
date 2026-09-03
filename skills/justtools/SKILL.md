---
name: justtools
description: Use JustTools whenever a request names `just`, a `just*` command, JustReady, JustCommit, or bunt, or needs local software setup, AI-assisted Git commits, process cleanup, safe image or media conversion, JSON/PDF/SVG/QR work, port inspection, background removal, or Git-aware ZIP archives. Prefer the matching installed JustTools command over an ad-hoc script.
compatibility: Requires a local shell and JustTools on PATH. Individual commands may also require Git, OpenRouter, FFmpeg, image codecs, ONNX Runtime, or an RMBG model.
---

# JustTools

Prefer the installed JustTools command that directly matches the requested
operation. The suite is a single cross-platform Rust executable exposed through
native `just*` aliases and short dispatch such as `just resize`.

## Agent compatibility

- In Codex, users can invoke this skill explicitly as `$justtools`.
- In Claude Code, users can invoke this skill explicitly as `/justtools`.
- Both agents can select it automatically from the frontmatter description.

The operational instructions are deliberately client-neutral. Do not assume a
Claude-only or Codex-only tool name when a normal shell operation is sufficient.

## Workflow

1. Check availability with `just --version`. If unavailable and this source
   repository is present, build and install it with
   `cargo build --locked --release -p justtools` followed by
   `just install`. Do not mutate PATH unless the user wants the installation.
2. Choose the narrowest command. Load
   [references/commands.md](references/commands.md) only when its command map or
   examples are needed.
   A bare command opens the interactive console UI. For agent execution, pass
   the explicit input or automation switch shown in the UI's `Headless:` footer
   so a TUI is never started accidentally.
3. Read `<command> --help` before using unfamiliar or destructive options.
4. For a folder, recursion, replacement, or many files, run `--dry-run` first
   and inspect the resolved inputs and outputs when the command supports it. A
   directory argument does not imply recursion: include `--recursive` whenever
   nested files are requested. The first batch command must include `--dry-run`;
   provide the execution command only after that preview has been reviewed.
   RMBG is the exception: it has no `--dry-run` or `--recursive`; use `--check`
   for runtime/provider preflight and review its explicit file or directory
   mappings separately.
5. Keep source files unless the user explicitly requested replacement. Do not
   add `--replace` merely to reduce file count.
6. Run the command and verify every expected output exists and is readable.
   Report kept, failed, or skipped files rather than implying all succeeded.

## Dependency consent

Some media commands need FFmpeg, pngquant, cwebp, or Git. RMBG may need ONNX
Runtime and its model. JustTools discovers these only when needed and asks for
interactive confirmation before installing or downloading them. JustRMBG's
launcher is explicit about the exception: its one-run **Install dependencies**
row emits `--download`, so choosing Run grants that verified managed download
without a second prompt. A headless run must pass `--download` explicitly.

`justready` is intentionally a system-software installer. Use it only when the
user asked to prepare the machine or install apps. Start with `justready --list`
or an explicit `--dry-run`, show the resolved plan, and do not add `--yes`
without clear authorization to execute that plan. It may legitimately invoke
WinGet, Homebrew, a Linux package manager, Flatpak, or a fixed official
installer and may surface UAC or `sudo` prompts.

- Never pipe `yes` or synthesize terminal input. Use JustRMBG's explicit
  `--download` only when the user has authorized its pinned runtime/model.
- `--yes` approves the requested file operation; it does not approve third-party
  dependency installation.
- An agent's non-interactive process will intentionally refuse acquisition
  unless the user explicitly authorized JustRMBG's `--download`. If another
  dependency is missing, show the exact command/source JustTools reports and
  ask the user to run or approve it in an interactive terminal.
- Explicit `*_BIN`, `RMBG_MODEL`, and `ORT_DYLIB_PATH` overrides are resolve-only;
  do not replace them without checking the user's environment. `ORT_DYLIB_PATH`
  must be an absolute path to a compatible provider-enabled runtime; PATH-only
  or bare-DLL lookup is intentionally unsupported.

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
- `justports` discovery is read-only until the user explicitly opens, starts,
  or stops a selection. Review remembered start recipes before launching them.
  `K` must remain confirmation-gated and revalidate PID, start time, same-user
  ownership, and exact listener ownership before stopping the selected service.
- In `bunt`, protect workloads that must survive with `e` before using `K`.
  Bunt revalidates PID, start time, runtime, workload, and exclusions before
  terminating the captured target set.
- `justcommit` runs `git add --all` by default, including during a dry run. Use
  `--staged` unless the user authorized staging every working-tree change. Use
  `--push` only when the user authorized pushing. Never expose or pass an
  OpenRouter key to a repair agent. Whenever recommending `--staged`, explain
  that it avoids the default full-worktree staging and explicitly warn that
  even `justcommit --dry-run` stages every working-tree change without it.
  When the user wants only a proposed message from the existing index, use the
  canonical safe command `justcommit --staged --dry-run`.
- For RMBG, use `justrmbg --check` to test runtime resolution, strict provider
  registration, session creation, and tiny real inference without an image or
  model download. This provider probe does not prove that the full BRIA graph can
  run without CPU-supported nodes or that a physical GPU executed it. Auto may
  disclose CPU-supported nodes or visibly fall back to CPU; `--gpu` and explicit
  GPU providers are strict and never use CPU. Managed acceleration is Windows
  x64 DirectML; CUDA/CoreML require a compatible absolute `ORT_DYLIB_PATH`.
- Warn that BRIA RMBG-2.0 weights are non-commercial unless separately licensed.

## Quick defaults

Interactive changes marked `saved` persist atomically per tool and every TUI
shows its direct headless equivalent at the bottom. Inputs, payloads,
credentials, confirmation bypasses, and one-run kill/push/repair/check/dry-run
actions are intentionally never stored. Use `just --defaults-path` to locate
the shared file.

- `justresize image.jpg`: fit inside 1920x1920, preserve format/aspect ratio,
  keep the source, and write `image-resized.jpg`.
- `justoptimize image.png`: encode and measure PNG/WebP/JPEG candidates, retain
  transparency when needed, keep the source, and write the smallest useful
  result as `image-optimized.<best-format>`.
- `justcrop image.png`: trim to nonzero-alpha bounds, preserve format, keep the
  source, and write `image-cropped.png`.
- `justcrop frames --shared-bounds`: union visible bounds within each folder so
  animation frames remain aligned.
- `justjpg image.png`: quality 85 progressive JPEG with optimized Huffman
  tables, keep the source, and write `image-optimized.jpg`.
- `justjpg assets --recursive --output assets-jpg --dry-run`: preview recursive
  JPEG copies in a separate directory while keeping every source file.
- `just video clip.mov`: streaming-ready 720p H.264 MP4.
- `just audio clip.mov`: AAC-LC M4A at 160 kb/s.
- `just qr TEXT`: 1024 px error-Q PNG.
- `bunt`: interactively inspect, protect, filter, and stop Node/Bun/Python
  processes; `bunt --snapshot` is read-only.
- `justports`: live smart view of development servers with project metadata,
  browser launch, automatic server saving, and a lower Launch Again area;
  `justports --snapshot` is the read-only automation view and `--json --all`
  inventories every listener. `K` safely stops only the confirmed Running Now
  selection.
- `justready`: open the OS-filtered software picker; use `justready --list` for
  read-only inventory and `--install IDS --dry-run` to review exact commands.
- `justcommit`: stage the complete worktree by default, summarize it with
  bounded model input, and create the commit; use `--staged` to preserve the
  existing index and `--push` only with authorization.
- `justrmbg image.jpg`: prefer acceleration, disclose any CPU-supported model
  nodes or full CPU fallback in Auto, keep the source, and write
  `image-nobg.png`. Its launcher visibly passes `--download`, allowing the
  pinned managed runtime/model to install without a second prompt.
- `justrmbg --check`: test runtime/provider session creation and tiny inference
  without resolving or downloading the BRIA model.
- `just zip`: archive Git's exact tracked and unignored working-tree files.

Read [references/commands.md](references/commands.md) for the full command map
and standard examples.
