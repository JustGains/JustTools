# bunt / justbunt

`bunt` is the JustTools terminal UI for finding, understanding, protecting, and stopping Node.js, Bun, and Python processes. The installed `bunt` and `justbunt` aliases and `just bunt` dispatch all run the same compiled `just` executable.

It is deliberately optimized for the common path: launch it, press `e` on anything that should survive future cleanups, then press `K` whenever you want to stop everything else.

## Install and run with JustTools

```powershell
cargo build --locked --release -p justtools
.\target\release\just.exe install
bunt
```

For a read-only check without opening the TUI:

```powershell
bunt --snapshot
```

## Fast workflow

| Key | Action |
| --- | --- |
| `e` | Instantly toggle the recommended persistent exclusion for the selected workload |
| `K` | Kill every currently non-protected Node/Bun/Python process |
| `x` | Kill only the selected process |
| `/` | Smart fuzzy filter |
| `Tab` | Cycle all, targets, and protected views |
| `1`–`4` | Show all runtimes, Node, Bun, or Python |
| `s` | Cycle name, CPU, memory, and age sorting |
| `?` | Show complete in-app help |

An exclusion created with `e` is based on the stable parts of a workload rather than its PID:

- runtime;
- project root or working directory;
- normalized entrypoint, module, or package script;
- executable path as a fallback when there is no project context.

That lets exclusions survive ordinary restarts while avoiding broad, surprising matches.

If the selected process is a worker beneath another Node/Bun/Python process, `bunt` automatically excludes the top runtime supervisor instead. That single rule is inherited by the supervisor's runtime descendants, so excluding `bun run dev` also protects the Node workers it launches. Global tools and cache-launched tools are normalized independently of the directory from which they were started.

## Smart filtering

Plain text is fuzzy and every term must match:

```text
/ vite
/ python api
/ jgains -test
```

Structured fields are also available:

```text
/ runtime:bun project:api
/ cmd:uvicorn -status:sleep
/ is:target
/ is:excluded
```

Supported fields are `runtime:`, `pid:`, `project:`, `cwd:`, `cmd:`, `status:`, and `is:`. Prefix any term with `-` to exclude it.

The default name sort is intentionally stable. CPU and memory sorts use small buckets, so insignificant metric noise does not reshuffle the process table on every refresh. Selection and scroll position remain anchored to the same PID/start-time identity while processes appear and disappear.

## Safety model

- The process ancestry that launched `bunt` is automatically safety-protected. This matters when `bunt` is launched from a Node- or Python-based shell/tool.
- Safety and persistent protection flow down to runtime child processes.
- Kill confirmation captures a snapshot. A process must still have the same PID, start time, runtime, and workload immediately before termination.
- Exclusions are checked again after confirmation.
- Processes that appear after confirmation are not silently added to the kill set.
- On Unix, `bunt` requests graceful termination, waits briefly, then force-kills survivors. On Windows, it uses native process termination because Unix signals are unavailable.
- Closing is non-blocking: the TUI shows animated stage progress, remains responsive throughout the grace period, and reports revalidated, graceful, forced, changed, and failed counts.
- Processes owned by another user or running at a higher integrity level may require elevation.

## Persistent configuration

Run this to locate the file:

```powershell
bunt --config-path
```

The TOML file is only created after the first exclusion is saved. It is human-editable. `bunt` creates workload-scoped rules automatically, but broader rules can be written directly:

```toml
version = 1

[behavior]
refresh_ms = 900
grace_period_ms = 1200
confirm_kill_all = true

[[excludes]]
id = "keep-project"
name = "Keep every runtime in this project"
scope = "project"
project = "f:/work/important-api"
created_at = 0

[[excludes]]
id = "keep-python"
name = "Keep every Python process"
scope = "runtime"
runtime = "python"
created_at = 0
```

Path matching is case-insensitive on Windows and case-sensitive on Unix-like systems.

## Details shown

The UI shows runtime, PID/parent PID, CPU, resident and virtual memory, age, status, executable, working directory, full command, project identity, workload identity, and recent process I/O. Some command or path information can be unavailable without sufficient OS permissions.
