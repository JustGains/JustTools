# JustTools console UI

Every bare JustTools command opens a consistent full-screen console. For
example, `justjpg`, `justresize`, `justpdf`, and `justcommit` open a settings
launcher; `justports`, `bunt`, and `justready` keep their purpose-built live
dashboards. Bare `just` opens the same styled tool browser.

The command-line contract is unchanged when any explicit argument is present
or stdin/stdout is redirected. This keeps scripts, pipes, CI, aliases, and
short dispatch deterministic:

```sh
justjpg                         # interactive launcher
justjpg photo.png --quality 92  # direct/headless execution
just jpg photo.png --quality 92 # the same direct execution
cat data.json | justjson        # piped JSON; no UI
```

## Launcher controls

| Key | Action |
| --- | --- |
| `Up` / `Down`, `j` / `k` | Move between settings and the Run row |
| `Left` / `Right`, `Space` | Toggle, cycle, or step the selected setting |
| `Enter` | Edit text/numbers, save an edit, or execute from Run |
| `Esc` | Cancel an edit; from the main view, exit without running |
| `D` | Reset every saved default for the current tool |
| `?` | Show launcher help and the defaults-file location |
| `q` | Exit without running |

The last line always starts with **Headless:** and updates live. It is the
copyable non-UI form of the selected operation. Required values are called out
until they are supplied. The terminal is restored before the command runs, so
normal progress, prompts, and errors remain readable.

Image and media launchers also keep two outcome lines visible under the selected
setting. **Output** resolves whether the result is beside each source or under
the chosen destination folder and shows the filename pattern. **Overwrite**
states whether sources are kept, atomically replaced, or removed only after a
new result is safe. In particular, blank-output JustWebP/JustAVIF runs remove a
source only when the new beside-source result is smaller; JustOptimize keeps
sources unless **Replace sources** is explicitly enabled.

| Tool | Blank output setting | Output folder set | Source behavior |
| --- | --- | --- | --- |
| `justpng` | Optimize the same `<name>.png` path | `<folder>/<name>.png` | In-place only when smaller; folder output keeps source |
| `justwebp` | `<source-folder>/<name>.webp` | `<folder>/<name>.webp` | Blank output removes source only after a smaller WebP is safe; folder output keeps it |
| `justavif` | `<source-folder>/<name>.avif` | `<folder>/<name>.avif` | Blank output removes source only after a smaller AVIF is safe; folder output keeps it |
| `justjpg` | `<name>-optimized.jpg` | `<folder>/<name>.jpg` | Keeps source unless **Replace sources** is on |
| `justoptimize` | `<name>-optimized.<best>` | `<folder>/<name>.<best>` | Keeps source unless **Replace sources** is on; already-smallest web inputs are left alone |
| `justresize` | `<name>-resized.<same>` | `<folder>/<name>.<same>` | Keeps source unless **Replace sources** is on |
| `justcrop` | `<name>-cropped.<same>` | `<folder>/<name>.<same>` | Keeps source unless **Replace sources** is on |
| `justrmbg` | `<name>-nobg.png` beside input | Exact file for one input; directory for a batch | Always keeps input; existing output is replaced atomically |

Multiple file/folder fields use semicolons between entries. Quote an entry when
the semicolon itself belongs to a filename:

```text
photo one.png;photo-two.png;"archive;final.png"
```

## Saved defaults

Rows marked `saved` are written immediately after a toggle, step, cycle, or
completed edit. The next bare invocation of that tool starts with those values,
and its Headless line includes every saved override. Returning a value to its
built-in setting removes that override instead of duplicating it in the file.

JustTools stores the versioned TOML file in the platform's per-user JustTools
configuration directory. Print the exact path with:

```sh
just --defaults-path
```

Set `JUSTTOOLS_DEFAULTS` to an explicit file path when an isolated or portable
configuration is needed. Writes are atomic. An invalid or newer unsupported
file fails visibly rather than silently discarding user choices.

Inputs and payloads are operation data, not defaults, so file lists, repository
paths, port numbers, PDF lists, and QR text reset between launches. Credentials
are never shown or stored. One-run safety/action switches also reset: dry run,
kill, push, repair, runtime check, and confirmation bypasses. `--yes` is not
offered by the launcher; normal confirmation gates remain active.

The following settings are remembered when changed:

| Tool group | Saved settings |
| --- | --- |
| Image/media tools | Quality, speed/method, sample rate, bitrate, output, parallel jobs, recursion, target re-encoding, source handling |
| `justoptimize` | Web quality, output, replacement, parallel jobs, recursion |
| `justresize` | Maximum/width/height, crop, upscale, JPEG quality, output, replacement, jobs, recursion |
| `justcrop` | Bounds mode, alpha threshold, padding, output, replacement, jobs, recursion |
| `justjson` / `justsvg` | Formatting/optimization choices, output, recursion |
| `justpdf` | Operation, output, page range, rotation, recursion |
| `justport` | Whether UDP endpoints are included |
| `justqr` | Output, format, size, correction level, margin, colors |
| `justrmbg` | Provider, output, and optional model path; pinned-dependency download permission is visible but one-run |
| `justzip` | Output and compression mode |
| `justcommit` | Model, stage scope, patch sampling, timeout, repair-agent preference |
| `justports` | Development-only versus all-listeners view |

When two saved choices cannot coexist, the launcher makes the displayed
Headless command safe and valid. An output directory takes precedence over a
saved replace/remove-source choice; resize width/height take precedence over
maximum-square sizing; JSON `--get` suppresses incompatible write/check actions;
RMBG runtime check suppresses image/output/model values; and terminal QR output
suppresses a file path.

## Purpose-built dashboards

The same bottom-line convention applies to the live tools:

- JustPorts shows `justports --snapshot` or `justports --snapshot --all`; its
  `a` view toggle is saved immediately.
- bunt shows `justbunt --snapshot` for the read-only headless inventory.
- JustReady builds `justready --install ID[,ID...]` from the current selection,
  or shows `justready --list` when nothing is selected.

Purpose-built state remains separate from launcher defaults. JustPorts still
stores bounded launch history in its data file; bunt stores protection rules,
behavior, and its immediately saved view/runtime/sort defaults in its own
config; and JustReady selections remain intentionally one-run installation
targets.
