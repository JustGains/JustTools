# JustTools command reference

## Command map

| Need | Command | Default behavior |
| --- | --- | --- |
| Browse or dispatch | `just` | Lists every tool and offers Add To Path only when needed |
| Prepare a machine | `justready` | OS-filtered curated app picker; installed apps are read-only and dependencies are planned |
| Manage development processes | `justbunt` / `bunt` | Inspect and persistently protect Node/Bun/Python workloads; guarded termination is optional |
| Summarize and commit staged changes | `justcommit` | Bounded OpenRouter digest/message, then `git commit`; the full diff is never uploaded |
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
| Remove backgrounds | `justrmbg` / `rmbg` | Local BRIA RMBG-2.0 inference to transparent PNG |
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
git add src tests
justcommit
justcommit --dry-run
justcommit --all
justcommit --model google/gemini-3.1-flash-lite
justcommit --repair
```

Set `OPENROUTER_API_KEY` or pass `--api-key`. JustCommit reads
`.cursor/rules/git-commit-structure.mdc` before `.gitmessage`, commits only the
staged index by default, checks that the index stayed unchanged, and keeps model
input fixed-size even when hundreds of thousands of paths changed. `--all`
stages the complete working tree before analysis, including in a dry run.

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
