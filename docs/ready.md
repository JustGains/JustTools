# JustReady / justready

JustReady is the JustTools terminal installer for preparing a useful Windows,
macOS, or Linux machine with very little ceremony. It is a curated setup tool,
not a generic package-store browser: every entry has a known native source, a
clear purpose, installed-state probes, and an OS support boundary.

The `justready` alias and `just ready` dispatch run the same compiled `just`
executable.

## Fast path

From a new machine, install JustTools and immediately open the picker:

```powershell
irm https://raw.githubusercontent.com/JustGains/JustTools/main/ready.ps1 | iex
```

```sh
curl -fsSL https://raw.githubusercontent.com/JustGains/JustTools/main/ready.sh | sh
```

The bootstrap chooses the matching x64/ARM64 GitHub release archive, verifies
its published SHA-256 sidecar, checks that the binary supports JustReady, uses
the suite's transactional alias installer, and then launches the TUI. It does
not require Rust, Node.js, Python, or a package manager.

If JustTools is already installed:

```sh
justready
```

The footer continuously translates the selection into
`justready --install ID[,ID...]`; with no selected apps it shows
`justready --list`. Selections are installation targets for this run and are
not persisted as defaults. See the shared [console UI guide](console-ui.md) for
the suite-wide footer and persistence contract.

The picker opens immediately while installed software is scanned in a
background thread. Press `r` to choose every missing recommendation, review the
plan with `Enter`, then confirm. JustReady closes the TUI before starting
installers so package-manager output, UAC, `sudo`, password prompts, and errors
remain visible in the normal terminal.

Installed entries cannot be selected and are removed from an existing
selection if the background scan discovers them. The catalog itself is static
during a run, so detection never reorders the list or moves the cursor.

## Keys

| Key | Action |
| --- | --- |
| `↑` / `↓`, `j` / `k` | Move through the stable catalog |
| `Space` | Select or clear the highlighted missing app |
| `r` | Select every missing recommended app |
| `a` | Select or clear every missing app |
| `/` | Search names, IDs, descriptions, and sections |
| `c` | Clear an applied search |
| `Tab` / `Shift-Tab` | Jump to the next or previous section |
| `Enter` | Review the dependency-expanded installation plan |
| `?` | Show in-app help and scan notes |
| `q` | Exit without changes |

## Recommended starter set

The recommended set is opinionated and OS-aware:

- Git, GitHub CLI, and GitHub Desktop on Windows/macOS;
- Codex CLI, Claude Code, and the Claude desktop app on Windows/macOS;
- Zed;
- .NET SDK 10;
- Notion on Windows/macOS;
- Telegram;
- DBeaver Community;
- ShareX, Windhawk, and Everything on Windows.

Unsupported choices are absent rather than disabled. For example, GitHub
Desktop, Notion, and Claude are offered on Windows and macOS, while ShareX,
Windhawk, and Everything are Windows-only.

Additional sections cover password managers and networking, local AI, editors
and terminals, language runtimes and containers, API/database clients,
communication, browsers, utilities, recording, and design tools.

## CLI and automation

```sh
justready --list
justready --json
justready --install git,github-cli
justready --install codex,claude-code,zed --dry-run
justready --recommended --dry-run
justready --recommended --yes
```

`--list` is human-readable and grouped by section. `--json` returns the current
platform, scan warnings, and every OS-available app with its ID, installed
state, recommendation flag, installer type, and package identity.

`--install` accepts comma-separated IDs and can be repeated. Friendly aliases
include `github` for GitHub Desktop, `gh` for GitHub CLI, `claude` for Claude
Code, and `claude-app` for the Claude desktop app. `--recommended` can be
combined with explicit app IDs.

## Bootstrap options

The remote bootstrap has safe environment overrides for pinned, unattended, or
repository-local use:

| Variable | Behavior |
| --- | --- |
| `JUSTTOOLS_VERSION` | Use a specific release tag, with or without the leading `v` |
| `JUSTTOOLS_BIN` | Install aliases in this directory |
| `JUSTREADY_NO_RUN=1` | Install only; do not open the TUI |
| `JUSTTOOLS_ARCHIVE` | Use a local `.zip` or `.tar.gz` and adjacent `.sha256` file |
| `JUSTTOOLS_NO_PATH=1` | Do not update the user's shell path |
| `JUSTTOOLS_SKIP_VERIFY=1` | Skip archive checksum verification; intended only for local development |

For example, a release archive produced in the repository can exercise the
exact Windows bootstrap without touching `PATH`:

```powershell
$env:JUSTTOOLS_ARCHIVE = (Resolve-Path dist/justtools-windows-x64.zip).Path
$env:JUSTTOOLS_BIN = "$env:TEMP/justtools-bin"
$env:JUSTREADY_NO_RUN = "1"
$env:JUSTTOOLS_NO_PATH = "1"
./ready.ps1
```

`--dry-run` performs the real installed-state scan and prints every planned
command without changing the system. Non-interactive execution requires
`--yes`; without it, JustReady asks once in a real terminal. A redirected
default invocation fails with instructions instead of trying to open a TUI.

## Installer routing

| Platform | Preferred sources | Automatic prerequisites |
| --- | --- | --- |
| Windows | Exact WinGet package IDs | Repair/install WinGet through Microsoft's PowerShell module flow |
| macOS | Homebrew formulae and casks | Non-interactive official Homebrew bootstrap |
| Linux | apt, dnf, pacman, zypper, or apk; Flathub; official scripts | Flatpak, Flathub user remote, `curl`, and `bash` when needed |

Linux GUI apps use the per-user Flathub remote and normally need no root access.
Native distro packages are combined into one package-manager operation to
reduce `sudo` prompts. Official scripts are first downloaded to a temporary
file with `curl`, then executed by `sh` or `bash`; fixed official URLs are shown
in the plan. The .NET Linux installer also exposes `dotnet` beside the running
JustReady executable, so the SDK is available anywhere JustTools already is.

Claude Code automatically adds Git when Git is absent. Installer
infrastructure is always ordered before dependent apps. Failure of WinGet,
Homebrew, native package setup, or Flathub configuration stops the dependent
plan; an individual independent app failure is reported while later apps can
continue.

## Detection and completion

Windows uses one bounded WinGet inventory pass plus command probes and known
display-name fallbacks for software registered outside WinGet. macOS uses one
Homebrew formula and cask inventory. Linux combines its distro package list,
Flatpak application list, and command probes. Inventory commands have a timeout
and fall back to command detection instead of blocking the interface forever.

After all installers finish, JustReady scans again and reports verified apps,
installer failures, and apps awaiting registration or a fresh shell. Some
per-user command installers update shell startup files, so opening a new
terminal can be necessary even after a successful install.

JustReady installs applications but does not launch them, sign into accounts,
accept product-specific licenses on the user's behalf, or silently upgrade
software it already detects.
