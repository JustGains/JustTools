# JustTools agent instructions

## Product contract

- Preserve one portable Rust binary with native `just*` aliases and short
  dispatch through `just <tool>`.
- A supported command with no arguments opens its console UI. Any explicit
  argument or redirected input keeps deterministic headless behavior.
- Keep launchers visually consistent: saved settings are labeled, destination
  and overwrite behavior are explicit, and the footer shows the exact
  `Headless:` command.
- Persist changed defaults atomically. Never save inputs, payloads, credentials,
  confirmation bypasses, or one-run actions such as kill, push, repair, check,
  download, or dry run.
- Preserve source files unless replacement is explicit. Reject ambiguous output
  collisions before processing and install outputs atomically.

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

Exercise changed commands through both their bare launcher and explicit
headless form. For image operations, verify paths, formats, dimensions, alpha,
source retention, and overwrite behavior. Before pushing a version tag, require
a green six-platform branch matrix. Then verify the tagged run publishes all six
archives and their checksum sidecars, with `docs/`, `AGENTS.md`, and
`skills/justtools/` present in every archive.
