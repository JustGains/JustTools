# JustPaste

`justpaste` turns a link into a file in the current folder. It works out what
the link is and takes the shortest route to the real file.

```sh
justpaste https://example.com/photo.jpg            # saves photo.jpg
justpaste https://www.youtube.com/watch?v=ID       # saves "<title> [<id>].mp4"
justpaste https://www.tiktok.com/@user/video/ID    # same, through yt-dlp
justpaste --clipboard                              # every link in the copied text
justpaste -o downloads https://a.test/x.pdf https://b.test/clip
justpaste --dry-run --clipboard                    # list the links; no network
```

On Windows, right-click a folder or its empty background and choose
**JustTools ▸ JustPaste · download the copied link here**; see the
[context menu guide](context-menu.md).

## How a link is handled

1. **The link is requested.** If the answer is a file of any kind (image,
   video, audio, PDF, archive, anything that is not a web page), it is saved
   as-is. Nothing is converted.
2. **A web page goes to yt-dlp**, which supports YouTube, TikTok, Instagram,
   X, Reddit, Vimeo, and most other video and social sites, plus a generic
   extractor for pages that simply embed a video. A link the plain request
   could not fetch is tried through yt-dlp as well.
3. **If yt-dlp finds nothing**, the video or image the page declares for link
   previews (`og:video`, `og:image`, and the Twitter equivalents) is saved.
4. Otherwise the link is reported as failed and the run exits with status 1.
   Other links in the same run are still processed.

yt-dlp downloads the highest resolution available, prefers H.264 and MP4 at
that resolution so the result plays everywhere, and merges into an `.mp4`. A
playlist link downloads one entry unless `--playlist` is passed. To shrink or
convert the result, run `justvideo`, `justmp3`, or another tool on it, or give
those tools the link directly.

## Names and safety

| Source | Saved name |
| --- | --- |
| Direct file | The server's `Content-Disposition` name, else the last part of the address; an extension is added from the content type when the name has none |
| yt-dlp | `<title> [<id>].<ext>` |

- Nothing is overwritten. A taken name is saved as `name (2).ext`, `name (3).ext`.
- A download is written under a hidden `.justpaste-*` name inside the
  destination and renamed only when complete, so an interrupted run leaves
  nothing behind under a real name.
- Names are stripped of characters and reserved words that a file system
  refuses.
- `--dry-run` only lists the links and the destination; it makes no request.

## Inputs

Links can be given as arguments, piped on standard input, or read from the
clipboard with `--clipboard`. Text around a link is ignored, so a copied
sentence or a list works; a bare `www.` address is treated as HTTPS. Anything
that is not an `http(s)` link is a usage error.

Clipboard access is built in on Windows, uses `pbpaste` on macOS, and uses
`wl-paste`, `xclip`, or `xsel` on Linux.

## Dependencies and privacy

Direct file links need nothing. Page links need yt-dlp and FFmpeg, which are
downloaded automatically when they are not on `PATH`, and yt-dlp is updated
whenever twelve hours have passed; both are described in the
[downloads guide](downloads.md). A link is fetched from the address you give
it, so the site sees the request. Only download what you have the right to.

Bare `justpaste` opens the launcher. A blank **Links** row pastes the
clipboard; **Output folder** and **Download playlists** are saved, and the
links themselves never are.
