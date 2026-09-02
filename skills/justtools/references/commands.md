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
| Optimize PNG | `justpng` | pngquant quality 65-90; replace only when smaller |
| Convert to WebP | `justwebp` | Quality 82/method 5; replace only when smaller |
| Convert to AVIF | `justavif` | AV1 quality 60/speed 6; replace only when smaller |
| Optimize video | `justvideo` | 720p H.264 MP4, CRF 28, AAC 128 kb/s |
| Extract/convert audio | `justaudio` | AAC-LC M4A, 160 kb/s, 48 kHz |
| Create MP3 | `justmp3` | LAME VBR quality 2, 48 kHz |
| Create WAV | `justwav` | Stereo 16-bit PCM, 48 kHz |
| Work with JSON | `justjson` | Format, validate, query, or minify |
| Work with PDF | `justpdf` | Inspect, merge, split, extract, or rotate |
| Optimize SVG | `justsvg` | Conservative SVGOMG-style OXVG optimization |
| Generate QR | `justqr` | 1024 px PNG, error correction Q, four-module margin |
| Inspect ports | `justport` | Show listener/process ownership; guarded kill is optional |
| Remove backgrounds | `justrmbg` / `rmbg` | Local BRIA RMBG-2.0 inference to `<name>-nobg.png`; Auto visibly falls back to CPU |
| Archive a repository | `justzip` | ZIP Git's tracked and unignored file set |

Every direct alias also works through short dispatch: `just resize`, `just pdf`,
`just rmbg`, and so on.

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
of every non-protected target. Launcher ancestry is always safety-protected.

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

## Background removal examples

```sh
justrmbg portrait.jpg
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
missing runtime. Normal inference may offer verified runtime/model acquisition
only after interactive consent. BRIA RMBG-2.0 weights are non-commercial unless
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

## Structured and document examples

```sh
just json data.json
just json data.json --query user.name
just pdf report-a.pdf report-b.pdf
just svg icon.svg
just qr "https://example.com" --output link.png
just port 4321 --json
```

## Verification

After processing, verify the output path, nonzero size, and appropriate format
or dimensions. For destructive operations, also verify the intended source was
the only file replaced or removed.
