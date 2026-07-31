# JustTools

Source of truth for the `just*` command-line tools. The deployed copies live in
`C:\cmd\bin` (on PATH); edit them **here** and deploy with:

```
bun run update
```

The script copies every file in `tools/` to `C:\cmd\bin` byte-for-byte and
reports updated/unchanged per file. If a bin copy was edited in place (differs
and is newer than the repo copy) it is skipped with a warning so the edit is
not lost; salvage it into `tools/` or re-run with `bun run update --force`.

## The tools

| Tool | What it does |
| --- | --- |
| `just` | interactive selector menu for the tools below (Node) |
| `justpng` | lossy-optimize PNGs in place with pngquant, parallel |
| `justvideo` | web-optimize videos to 720p H.264 MP4 via ffmpeg, parallel |
| `justwebp` | convert images to lossy WebP via cwebp, parallel |
| `justzip` | zip a git working tree honoring all .gitignore rules (PowerShell) |

Each tool ships as an extensionless bash script (Git Bash) plus a `.cmd` shim
for cmd/PowerShell; `just` is Node (`just.js`), `justzip` is PowerShell
(`justzip.ps1`). Shims resolve their siblings relative to their own location,
so everything must deploy to the same folder. `just` discovers tools by
scanning that folder for `just[a-z0-9]+` files and reads each script's header
comment as its live help text.

Line endings are load-bearing: `.cmd` files are CRLF, bash scripts are LF.
`.gitattributes` disables git EOL conversion repo-wide to protect both; keep
that in mind if you add files.

Adding a tool: drop the script (+ shim) in `tools/`, run `bun run update`.
The `just` menu picks it up automatically.
