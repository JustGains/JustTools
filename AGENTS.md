# JustTools agent instructions

## Product contract

- Preserve one portable Rust binary with native `just*` aliases and short
  dispatch through `just <tool>`. The only companion is the Windows File
  Explorer extension, `justtools_shell.dll`, which holds no tool logic.
- A supported command with no arguments opens its console UI. Any explicit
  argument or redirected input keeps deterministic headless behavior.
- Keep launchers visually consistent: saved settings are labeled, destination
  and overwrite behavior are explicit, and the footer shows the exact
  `Headless:` command.
- Persist changed defaults atomically. Never save inputs, payloads, credentials,
  confirmation bypasses, or one-run actions such as kill, push, repair, check,
  download, or dry run.
- Download only yt-dlp and FFmpeg without asking, only into the per-user
  JustTools folder, and only after verifying the vendor's published SHA-256.
  Every other dependency keeps its interactive confirmation.
- Preserve source files unless replacement is explicit. Reject ambiguous output
  collisions before processing and install outputs atomically.
- Describe the File Explorer menu only in `explorer/menu`. A menu entry must be
  an ordinary headless command or a launcher, must keep its source unless its
  label says "in place", and must never pass `--yes`. Keep the menu one level
  deep: Windows 11 leaves a nested flyout empty. The extension runs inside
  Explorer and must not be able to panic.

## Changes

- Keep Rust 1.90 compatibility and use locked dependencies.
- Prefer shared launcher, preference, path, and batch helpers over per-command
  variations.
- Update the relevant guide in `docs/`, the concise `README.md`, and
  `skills/justtools/references/commands.md` when user-visible behavior changes.
  Update `skills/justtools/SKILL.md` when agent safety or workflow changes.
- Do not commit secrets, model files, runtimes, generated target files, or local
  defaults.

## Verification

Run the narrowest focused tests while iterating, then before release run:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo +1.90.0 check --locked --workspace --all-targets
cargo build --locked --release -p justtools
```

On Windows, also build `-p justtools-shell`.

Exercise changed commands through both their bare launcher and explicit
headless form. After changing the context menu, run `just context install` and
right-click a matching file in both the Windows 11 menu and "Show more options". For image operations, verify paths, formats, dimensions, alpha,
source retention, and overwrite behavior. Before pushing a version tag, require
a green six-platform branch matrix. Then verify the tagged run publishes all six
archives and their checksum sidecars, with `docs/`, `AGENTS.md`, and
`skills/justtools/` present in every archive and `justtools_shell.dll` in both
Windows archives.
