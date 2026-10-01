# Media downloads

`justvideo`, `justaudio`, `justmp3`, and `justwav` accept an `http(s)` URL
anywhere they accept a file. yt-dlp downloads it into a temporary folder, the
tool converts it with its own settings, and only the converted result is kept.
A URL therefore produces exactly what the same tool produces from a local file.

```sh
justvideo https://example.com/watch?v=ID          # ./<title> [<id>].mp4, 720p CRF 28
justvideo --resolution 4k https://example.com/watch?v=ID   # up to 2160p
justaudio https://example.com/watch?v=ID          # ./<title> [<id>].m4a, AAC-LC 160k
justmp3 https://example.com/watch?v=ID -o mixes   # mixes/<title> [<id>].mp3
justvideo --playlist https://example.com/list=ID  # every entry of a playlist
justaudio --dry-run https://example.com/watch     # plan only; nothing downloads
justvideo clips https://example.com/watch?v=ID    # local folder and URL in one run
```

## What is downloaded

| Tool | Selected source |
| --- | --- |
| `justvideo` | Best video plus audio at 1080p or lower (or at a larger `--resolution`), merged to a temporary MKV |
| `justaudio`, `justmp3`, `justwav` | Best audio-only stream |

The cap exists because the result is re-encoded to `--resolution` (720p by
default), so a taller source would only cost bandwidth. `--resolution 1440p` and
`4k` raise the cap to match, and `--resolution source` removes it. Playlists are skipped unless `--playlist` is passed:
a `watch?v=...&list=...` link downloads the one video. yt-dlp runs with
`--ignore-config`, so a personal yt-dlp configuration cannot redirect the
output or re-encode behind the tool's back.

## Output

Downloads have no source folder to sit beside, so they are written to the
current folder, or to `--output`, as `<title> [<id>].<target-extension>`.
`justvideo` uses that plain name rather than the `-web` suffix reserved for
optimizing a local file. An existing destination is replaced atomically,
exactly as it is for a local conversion.

The downloaded file itself is temporary and is removed with the run, so
`--replace` has nothing to remove for a URL. Local files and URLs can be mixed
in one command; two inputs that would write the same output are rejected before
anything is encoded. Each line of the summary names the URL it came from:

```
justaudio: downloading https://example.com/watch?v=ID
justaudio: 1 file(s), 2 job(s)
  done    https://example.com/watch?v=ID -> Title [ID].m4a  12.4 MiB -> 3.1 MiB
```

## Keeping yt-dlp current

Every run of `justvideo`, `justaudio`, `justmp3`, `justwav`, or `justpaste`
checks when yt-dlp was last updated and runs `yt-dlp --update` if twelve hours
have passed, whether or not that run downloads anything. Sites change faster
than releases reach package managers, so this keeps downloads working. A dry
run and a machine without yt-dlp are left alone. The timestamp of the last
attempt is stored next to the defaults file, which `just --defaults-path`
prints, as `yt-dlp-update`.

The update is best-effort. A package-managed yt-dlp refuses to replace itself
and an offline machine cannot reach the release feed; either way the message is
a warning and the run proceeds with the installed version. The next attempt
still waits twelve hours, so a broken feed cannot slow every run.

## Requirements

yt-dlp and FFmpeg are used from `PATH` when they are there. When one is
missing, JustTools downloads it automatically, on every platform, the first
time a tool needs it:

| Program | Source | Verified against |
| --- | --- | --- |
| yt-dlp | The official `yt-dlp/yt-dlp` GitHub release for this OS and CPU | The release's `SHA2-256SUMS` |
| FFmpeg and ffprobe, Windows | Gyan Doshi's release essentials build (the build WinGet's `Gyan.FFmpeg` installs) | The `.sha256` published beside it |
| FFmpeg and ffprobe, macOS and Linux | Martin Riedl's static release builds for x64 and ARM64 | The `.sha256` published beside each |

The programs go into a per-user folder (`%LOCALAPPDATA%\JustTools\deps\bin`
on Windows, the matching data folder elsewhere). Nothing is installed
system-wide, `PATH` is not changed, no elevation is requested, and a download
that fails verification is discarded. A copy on `PATH` always wins. A managed
yt-dlp updates itself in place through the twelve-hour check; a managed FFmpeg
is refreshed with `just deps fetch ffmpeg`.

```sh
just deps fetch yt-dlp ffmpeg   # download or refresh the managed copies now
```

Set `JUSTTOOLS_NO_DOWNLOAD=1` to turn automatic downloads off, and
`JUSTTOOLS_DEPS_DIR` to choose the folder. If a download is off or fails, the
previous flow applies: JustTools shows the exact install command for WinGet,
Homebrew, APT, DNF, or pacman and runs it only after interactive confirmation.

Set `YTDLP_BIN` to pin a specific executable, the same way `FFMPEG_BIN` pins
FFmpeg. An explicit path is resolved as-is and never triggers a download or an
installation.

## Console

Run `justvideo` or `justaudio` with no arguments and type or paste the URL into
**Input files / folders / URLs**; separate several with semicolons. The footer
shows where the download will land and the exact `Headless:` command. **Download
playlists** is a saved setting; the URL itself is never saved.
