# JustPorts

JustPorts is a live terminal dashboard for local development servers. Run
`justports` or `just ports` to open it.

Its last line shows the equivalent read-only headless command:
`justports --snapshot` in the development view or
`justports --snapshot --all` in the all-listeners view. Pressing `a` changes
the view and immediately saves that choice as the next bare-run default. Toggle
back to restore the development-only default; the shared file is described in
the [console UI guide](console-ui.md).

## What it discovers

Every refresh combines the operating system's listening TCP sockets with the
owning process name, PID, command line, executable, working directory, memory,
and age. It searches up to 20 parent directories for `package.json`,
`Cargo.toml`, `pyproject.toml`, .NET project files, Go, Ruby, PHP, and common
lock/project markers. The nearest match supplies the project name and root.

Known process commands and package metadata label common stacks. The default
view shows listeners that look like development servers; press `a` to audit all
listeners. High dynamic ports owned by a process that already has a stable
listener are treated as companion/internal listeners instead of browser-ready
servers.

Wildcard binds are opened through `localhost`. Loopback addresses are preferred
over LAN addresses, IPv6 URLs are bracketed correctly, and common HTTPS flags
and ports select `https://` automatically.

## Interactive controls

| Key | Action |
| --- | --- |
| `Up` / `Down`, `j` / `k` | Move in the focused panel |
| `Tab` | Switch between Running Now and Launch Again |
| `Enter` | Open a running URL or start a saved project again |
| `o` | Open the selected active or remembered URL |
| `p` | Open the detected project folder |
| `K` | Confirm and stop the selected Running Now service |
| `/` | Filter by project, URL, port, stack, process, command, or path |
| `a` | Toggle smart dev servers and every TCP listener |
| `r` | Refresh immediately; the normal interval is two seconds |
| `?` | Show in-app help |
| `q` / `Ctrl+C` | Quit |

## Automatic saving and Launch Again

Every detected development server is saved automatically during normal live
refreshes; the user never needs to create a snapshot manually. The lower
**Launch Again** area contains up to 40 saved servers that are not active now.
JustPorts prefers a package script named `dev`, `start`,
`serve`, or `web` and selects Bun, pnpm, Yarn, or npm from the project's lock
file. It also understands common .NET, Cargo, Python, Go, and PHP starts. When
no common recipe exists, a prior process command is remembered only when it
passes conservative credential and opaque-token checks.

Launching again is always explicit. JustPorts shows the exact recipe, runs it from the
remembered project directory, detaches its standard streams, and waits for the
normal live refresh to discover the new listener. It does not terminate or
replace existing processes.

Stopping is also explicit. `K` opens a confirmation for the selected Running
Now service. Immediately before termination, JustPorts rechecks the PID,
process start time, same-user ownership, and that the process still owns the
selected listening port. System processes, changed identities, stale port
owners, and services owned by another user are refused. Other listeners and
saved Launch Again entries are untouched.

The JSON history file is written atomically in the per-user JustTools data
directory. Run `justports --history-path` to print its exact location. Command
arguments whose names suggest tokens, API keys, passwords, secrets, or
credentials are redacted from live output and are never cached as launch
recipes. `JUSTPORTS_HISTORY` can select a different history file for portable
or isolated automation.

## Automation

```sh
justports --snapshot
justports --snapshot --all
justports --json
justports --json --all
justports --open 5173
```

`--snapshot`, `--json`, and `--open` are mutually exclusive. `--all` can be
combined with snapshot or JSON output. Both inventory modes feed the same
automatic bounded save history used by Launch Again.
