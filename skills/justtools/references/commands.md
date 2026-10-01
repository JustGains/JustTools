# JustTools command reference

## Command map

| Need | Command | Default behavior |
| --- | --- | --- |
| Browse or dispatch | `just` | Lists every tool and offers Add To Path only when needed |
| Prepare a machine | `justready` | OS-filtered curated app picker; installed apps are read-only and dependencies are planned |
| Manage development processes | `justbunt` / `bunt` | Inspect and persistently protect Node/Bun/Python workloads; guarded termination is optional |
| Stage, summarize, and commit changes | `justcommit` | Bounded OpenRouter digest/message, then `git commit` and optional `git push`; the full diff is never uploaded |
| Crop transparent borders | `justcrop` | Per-image or folder-wide shared alpha bounds, same format, keep source |
| Create optimized JPEG | `justjpg` | Quality 85 progressive 4:2:0, white alpha background, keep source |
| Resize still images | `justresize` | Fit within 1920x1920, no upscale, same format, keep source |
| Optimize PNG | `justpng` | pngquant quality 65-90; same path only when smaller |
| Convert to WebP | `justwebp` | Blank output removes source only after a smaller WebP is safe |
| Convert to AVIF | `justavif` | Blank output removes source only after a smaller AVIF is safe |
| Optimize video | `justvideo` | 720p H.264 MP4, CRF 28, AAC 128 kb/s; `--resolution` 480p to 4k or source; `http(s)` URLs download with yt-dlp |
| Extract/convert audio | `justaudio` | AAC-LC M4A, 160 kb/s, 48 kHz; `http(s)` URLs download with yt-dlp |
| Create MP3 | `justmp3` | LAME VBR quality 2, 48 kHz; `http(s)` URLs download with yt-dlp |
| Download any link | `justpaste` | A file link is saved as-is; a page goes through yt-dlp at the highest resolution as MP4; never overwrites |
| Choose the best web image | `justoptimize` | Measure PNG/WebP/JPEG; preserve alpha; keep source |
| Create WAV | `justwav` | Stereo 16-bit PCM, 48 kHz; `http(s)` URLs download with yt-dlp |
| Work with JSON | `justjson` | Format, validate, query, or minify |
| Work with PDF | `justpdf` | Inspect, merge, split, extract, rotate, save images, or list unique links |
| Extract links | `justlinks` | Unique links from PDF, Office, and text files to `links.txt`; `--csv` adds title, file, location, and count |
| Optimize SVG | `justsvg` | Conservative SVGOMG-style OXVG optimization |
| Generate QR | `justqr` | 1024 px PNG, error correction Q, four-module margin |
| Inspect ports | `justport` | Show listener/process ownership; guarded kill is optional |
| Show the public IP address | `justip` | Report the public IPv4 and IPv6 address together; `-4`/`-6` narrow it |
| Browse development servers | `justports` | Live smart TUI, automatic saving, and one-key Launch Again recipes |
| Remove backgrounds | `justrmbg` / `rmbg` | Local BRIA RMBG-2.0 inference to `<name>-nobg.png`; Auto visibly falls back to CPU |
| Archive a repository | `justzip` | ZIP Git's tracked and unignored file set |
| Create and enter a directory | `justmkcd` / `mkcd` | Create one directory; enter it through `just init` shell integration |
| Claude permission-bypass shortcut | `claude_` | Shell function for `claude --dangerously-skip-permissions`, forwarding arguments |
| Codex permission-bypass shortcut | `codex_` | Shell function for `codex --yolo`, forwarding arguments |
| Windows right-click menu | `just context install` | Per-user File Explorer menu; `status` and `uninstall` manage it |

Every direct alias also works through short dispatch: `just resize`, `just pdf`,
`just rmbg`, and so on.

Every bare direct alias opens a standardized full-screen console. Changed rows
marked `saved` persist immediately, while inputs, credentials, confirmation
bypasses, and one-run action/safety switches do not. The bottom line shows the
exact headless command. Supplying any explicit argument or piping stdin bypasses
the UI. `just --defaults-path` prints the shared defaults file; `D` resets the
current tool's saved overrides.

## Shell helpers

Windows installations include automatic PowerShell `.ps1` launchers for `mkcd`,
`justmkcd`, `just`, `claude_`, and `codex_`. No init or profile changes are needed,
including in `-NoProfile` sessions. Explicit `.exe` invocation bypasses them.
For other shells, load `eval "$(just init bash)"` in Bash,
`eval "$(just init zsh)"` in Zsh, or
`just init fish | source` in Fish. Add the matching line to the shell profile
to persist it. For an uninstalled PowerShell binary, manual setup is
`just init powershell | Out-String | Invoke-Expression`. `just init` prints
definitions and never edits the profile itself.

`mkcd "my folder"` creates and enters exactly one directory. `mkcd -p a/b`
creates missing parents and accepts an existing directory. Use `--` before a
leading-dash path. Failures never change location. Without the shell functions,
the native binary creates and prints the path but cannot move the parent shell.
`just mkcd --print-path DIR` is the explicit create-and-print automation form;
it never changes the shell directory. Paths are never saved by the bare UI.
In PowerShell, quote the separator: `mkcd '--' --folder`; an unquoted `--`
is consumed by PowerShell before it reaches a shell function.

`claude_` and `codex_` are shell-only shortcuts for the installed external CLIs;
they bypass permission prompts with the flags shown above and forward all
arguments. They have no JustTools launcher or short dispatch command. Use them
only when the user explicitly requests these modes. See `docs/mkcd.md`.

## Software setup examples

```powershell
irm https://raw.githubusercontent.com/JustGains/JustTools/main/ready.ps1 | iex
```

```sh
curl -fsSL https://raw.githubusercontent.com/JustGains/JustTools/main/ready.sh | sh
```

```sh
justready
justready --list
justready --json
justready --install git,github-cli --dry-run
justready --install dotnet,claude-app,notion --dry-run
justready --recommended --dry-run
```

The catalog is filtered before display for Windows, macOS, or Linux and grouped
by purpose. Use `--yes` only when the user has approved the displayed software
plan. JustReady restores the normal terminal before invoking native installers.
The remote bootstrap chooses and verifies the matching GitHub release archive,
installs JustTools transactionally, and opens JustReady.

## Process manager examples

```sh
bunt
bunt --snapshot
bunt --config-path
just bunt
```

Inside the TUI, `e` toggles a persistent workload exclusion, `/` opens smart
filtering, `x` stops the selected target, and `K` stops the revalidated snapshot
of every non-protected target. View, runtime-filter, and sort changes save as
the next defaults. Launcher ancestry is always safety-protected, and the footer
shows the read-only `justbunt --snapshot` headless form.

## Development server browser examples

```sh
justports
justports --all
justports --snapshot
justports --json --all
justports --open 5173
justports --history-path
just ports
```

Running Now joins TCP listeners to project metadata and opens the selected URL
or project folder. Every detected dev server is saved automatically. When it
stops, Launch Again offers the explicitly selected safe package/common/prior
recipe from its original directory. Commands containing likely credentials or
opaque tokens are not cached. Use `justport` instead when exact-port ownership
from scripts is the goal; `K` provides confirmation-gated termination for the
selected Running Now service.

## Public IP examples

```sh
justip
justip -4
justip -6
justip --plain
justip --json
justip --timeout 15
just ip
```

Both families are reported without a switch, so `-4` and `-6` are only needed
to narrow the result. The address is echoed back by a public lookup service, so
it is what the internet sees rather than a local interface address; each family
is pinned by hostname and an answer in the wrong family is discarded. A family
with no globally routable source address is reported as unavailable immediately
instead of waiting out the timeout, and one family answering still succeeds.
The run fails, with exit status 1 and nothing on stdout, only when no requested
family answers. `--plain` and `--json` keep stdout free of anything but the
answer.

## Commit examples

```sh
justcommit
justcommit --dry-run
justcommit --push
justcommit --staged
justcommit --model google/gemini-3.1-flash-lite
justcommit --repair
```

Set `OPENROUTER_API_KEY` or pass `--api-key`. JustCommit reads
`.cursor/rules/git-commit-structure.mdc` before `.gitmessage`, stages the complete
working tree by default, checks that the index stayed unchanged, and keeps model
input fixed-size even when hundreds of thousands of paths changed. `--staged`
uses only the existing index. `--push` pushes only after the commit succeeds.

## Resize examples

```sh
justresize photo.jpg
justresize photo.jpg --width 800
justresize photo.jpg --width 1200 --height 630 --crop
justresize photos --max 1600 --recursive --output resized --dry-run
justresize photos --max 1600 --recursive --output resized --yes
justresize avatar.png --width 512 --height 512 --crop --upscale
```

One dimension preserves aspect ratio. Width plus height defines a containing
box unless `--crop` is present. `--crop` makes an exact centered result and
requires both dimensions. Existing small images are skipped unless `--upscale`
is explicit.

## Crop and JPEG examples

```sh
justcrop logo.png
justcrop frames --shared-bounds --output cropped
justcrop sprites --recursive --padding 2 --output cropped --dry-run
justcrop sprites --recursive --padding 2 --output cropped --yes
justjpg photo.png
justjpg photos --recursive --quality 85 --output jpg --dry-run
justjpg transparent.png --background F5F5F5
```

Crop includes every nonzero-alpha pixel unless `--threshold` is raised. JPG
composites transparency onto white, uses progressive output, and strips
metadata by default. Both commands keep source files unless `--replace` is
explicit.

## Automatic web-image optimization

```sh
justoptimize hero.png --dry-run
justoptimize hero.png
justoptimize assets --recursive --output web --dry-run
justoptimize assets --recursive --output web --yes
```

JustOptimize is self-contained and needs no external encoder. It measures real
PNG, WebP, and eligible progressive JPEG outputs. Any non-opaque pixel excludes
JPEG, while PNG and WebP retain alpha. The default keeps the source and writes
`<name>-optimized.<best>` only when an encoded candidate is smaller than an
already-web-ready original. `--output` keeps sources and writes `<name>.<best>`
under that folder; `--replace` is the only source-removing mode.

## Background removal examples

```sh
justrmbg portrait.jpg
justrmbg portrait.jpg --download
justrmbg --check
justrmbg --check --gpu
justrmbg portrait.jpg --provider cpu
```

The default `auto` provider prefers acceleration. It reports when CPU supports
model nodes the accelerator cannot execute, and visibly reports any full move to
CPU. `--cpu` is strict CPU. `--gpu` is the strict platform GPU (DirectML on
Windows, CoreML on macOS, and CUDA elsewhere), while `--provider` also accepts
`auto`, `cpu`, `directml`, `cuda`, and `coreml`. Explicit GPU modes never fall
back to CPU.

`--check` creates a provider session and runs tiny real inference without an
image and without resolving or downloading the BRIA model. It proves provider
operation, not strict compatibility of the full BRIA graph or physical-GPU use.
Windows x64 can use
the verified managed DirectML bundle. CUDA and CoreML require an absolute
`ORT_DYLIB_PATH` naming a compatible provider-enabled runtime; relative and
PATH-only DLL lookup is rejected. A non-interactive run never downloads a
missing runtime unless `--download` is explicit. The bare launcher visibly
enables that one-run permission, so Run can fetch the pinned runtime/model
without another prompt. BRIA RMBG-2.0 weights are non-commercial unless
separately licensed.

Multiple explicit images form a batch and a supplied `--output` is then a
directory. RMBG continues after per-file failures, reports totals, and exits
nonzero if any file failed. It has no `--dry-run` or `--recursive`; use
`--check` as runtime preflight and review explicit file/directory mappings.

## Common batch pattern

```sh
justwebp assets --recursive --output optimized --dry-run
justwebp assets --recursive --output optimized --yes
```

Use the command-specific help because not every option applies to every tool.
`--output` generally keeps sources. `--replace` is explicit and destructive.
The launchers keep the resolved output pattern and overwrite/source policy
visible above the Headless command.

## Video resolution

```sh
justvideo clip.mov                       # 720p
justvideo clip.mov --resolution 1080p
justvideo clip.mov --resolution 4k       # 2160p
justvideo clip.mov --resolution source   # keep the source size
```

`--resolution` accepts `480p`, `720p`, `1080p`, `1440p`, `4k`, or `source` and
bounds the frame without ever upscaling. The output name stays `<name>-web.mp4`.

## Windows context menu

```powershell
just context install
just context status
just context uninstall
```

Adds a per-user **JustTools** entry to File Explorer for supported files and
folders. Entries run built-in defaults in a console window that prints the
Headless command; `…` entries open the launcher with the selection filled in.
The top-level Windows 11 menu needs a signed `justtools-shell.msix` or Developer
Mode; `--classic` registers under "Show more options". `just context run` is
the entry point Explorer uses and is not meant to be typed. See
`docs/context-menu.md`.

## Media download examples

```sh
just video https://example.com/watch?v=ID
just audio https://example.com/watch?v=ID --output podcasts
just mp3 --playlist https://example.com/list=ID
just video --dry-run https://example.com/watch?v=ID
```

`justvideo`, `justaudio`, `justmp3`, and `justwav` accept an `http(s)` URL
wherever they accept a file. yt-dlp downloads it to a temporary folder (best
video plus audio at 1080p or lower, or at a larger `--resolution`, for video; best audio-only stream otherwise)
and the tool then encodes it with its usual settings into the current folder, or
`--output`, as `<title> [<id>].<ext>`. The download is never kept, so `--replace`
has nothing to remove. A playlist link downloads one entry unless `--playlist`
is passed, and `--dry-run` reports the plan without any network access. yt-dlp
runs with `--ignore-config`. Every run of these tools and of `justpaste` runs
`yt-dlp --update` once twelve hours have passed; an update failure is a
warning, never a failed run. A missing yt-dlp or FFmpeg is downloaded
automatically into a per-user folder and verified against the vendor's SHA-256
(`JUSTTOOLS_NO_DOWNLOAD=1` disables this; `just deps fetch yt-dlp ffmpeg`
does it up front). See `docs/downloads.md`.

## Paste a link

```sh
justpaste https://example.com/photo.jpg
justpaste https://www.youtube.com/watch?v=ID
justpaste --clipboard
justpaste -o downloads --dry-run https://example.com/a.pdf
```

`justpaste` saves what a link points at into the current folder or `--output`.
A file answer is saved unconverted under the server's name; a page goes to
yt-dlp (YouTube, TikTok, and most other sites), and if that finds nothing the
page's declared `og:video`/`og:image` is saved. A taken name becomes
`name (2).ext`; nothing is overwritten and partial downloads leave nothing
behind. `--clipboard` takes every link in the copied text, `--playlist` opts
into multi-entry links, and `--dry-run` makes no request. Use `justvideo` or
`justmp3` with a URL instead when the result should be converted. See
`docs/paste.md`.

## Structured and document examples

```sh
just json data.json
just json data.json --get user.name
just pdf report-a.pdf report-b.pdf
just pdf images guide.pdf --dry-run
just pdf links guide.pdf --output links.txt
just svg icon.svg
just qr "https://example.com" --output link.png
just port 4321 --json
```

`justpdf images` writes `<name>-images/p<page>-<n>.<ext>` beside the PDF, or
into the `--output` folder. JPEGs are copied byte-for-byte unless real
transparency requires an RGBA PNG; raw images become PNG, and unsupported
encodings are reported as skipped. `justpdf links` writes clickable URLs and
bookmark URLs, deduplicated in first-seen order, to `<name>-links.txt` or the
`--output` file or folder. Both accept `--pages` and `--dry-run`. See
`docs/pdf.md`.

```sh
just links guide.pdf plan.xlsx --csv --dry-run
just links programs --recursive --csv -o all-links.csv --yes
just links notes.md -o -
```

`justlinks` reads PDFs (annotations, typed URLs, bookmarks), `.xlsx/.docx/.pptx`
(hyperlinks, HYPERLINK formulas and fields, typed URLs), and text formats, and
deduplicates across every input in first-seen order, ignoring casing throughout
the URL. YouTube watch, `youtu.be`, Shorts, live, and embed variants (including
mobile, music, and no-cookie hosts) become `https://www.youtube.com/watch?v=ID`
with no extra parameters or fragment. Video IDs also match without casing;
the first ID's spelling is retained. Other links retain their first spelling
and parameters. Default output is
`links.txt` (or `links.csv` with `--csv`) in the current folder; `-o -` prints.
CSV columns are `url,title,file,location,occurrences`; a title is the link's
text, a spreadsheet row's first text cell, or the label before a typed URL
such as `Squat: https://…`. Legacy `.xls/.doc/.ppt` are reported as skipped.
Files are read in parallel (`--jobs`, default CPUs up to 8) with
deterministic output. See `docs/links.md`.

## Verification

After processing, verify the output path, nonzero size, and appropriate format
or dimensions. For destructive operations, also verify the intended source was
the only file replaced or removed.
