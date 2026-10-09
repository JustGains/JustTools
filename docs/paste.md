# JustPaste

`justpaste` saves clipboard images, copies files, downloads links, and clones
Git repositories into the current folder. Windows also offers a per-user
Explorer Smart Paste tray mode.

The CLI and bare console launcher work on Windows, macOS, and Linux, on x64
and ARM64. URL arguments, repository links, `--copy`, and piped text work
without a desktop session. Explorer Ctrl+V integration and its tray/settings
controls are Windows-only.

```sh
justpaste https://example.com/photo.jpg            # saves photo.jpg
justpaste https://github.com/owner/repo             # clones into ./repo
justpaste git@github.com:owner/repo.git -o projects # clones into projects/repo
justpaste https://www.youtube.com/watch?v=ID       # saves "<title>.mp4"
justpaste https://www.tiktok.com/@user/video/ID    # same, through yt-dlp
justpaste --clipboard                              # every link in the copied text
justpaste -o downloads https://a.test/x.pdf https://b.test/clip
justpaste --dry-run --clipboard                    # list the links; no network
```

```sh
printf '%s\n' 'https://github.com/owner/repo' | justpaste -o projects
printf '%s\n' 'Save this text' | justpaste --text -o notes
just paste --copy project --output existing-folder --dry-run
```

Explicit arguments and piped input bypass the console launcher. Plain text
needs `--text`; repository and download links take priority over text saving.

On Windows, right-click a folder or its empty background and choose
**JustTools ▸ JustPaste · paste here**; see the
[context menu guide](context-menu.md).

## Smart Paste in Explorer (Windows)

```powershell
just paste --enable          # start now, no login change
just paste --status
just paste --settings        # open the native settings window
just paste --diagnose        # read-only focus and Explorer connection diagnostics
just paste --disable         # cancel any active paste, then stop
just paste --startup on      # optional login startup; off removes it
```

Copy a Git repository link and press Ctrl+V in Explorer's file pane to clone
into that folder. For example, `https://github.com/owner/repo` creates a `repo`
subfolder. The right-click JustPaste action and `--clipboard` use the same
repository detection.

With Explorer's filesystem file pane focused, **Ctrl+V** saves a clipboard
image as PNG or downloads copied links into the active tab's folder. Actual
image data takes priority over a browser's accompanying URL. Address-bar,
search, rename, other-app, and unsupported/virtual-folder pastes stay native.
The destination and clipboard are checked at activation; switching folders
during a download does not redirect its output. There is no clipboard history
or copy-time downloading.

Click the violet-and-blue tray icon to open **JustPaste settings**, or run
`just paste --settings`. The native window has copy filters, custom ignored
paths, text-to-file, and sign-in startup. **Save changes** validates the paths
and saves the filters together; **Cancel** leaves them unchanged. Saved fields
take effect without restarting Smart Paste. Settings also opens when the
resident is stopped and does not need a console.

Right-click the icon for **Open settings…**, **Pause Smart Paste**, the copy
filters, **Start at sign-in**, **Open last folder**, **Cancel current paste**,
and **Quit JustPaste**. The menu groups related actions and disables Open last
folder and Cancel current paste when they do not apply. Action in the bare
console launcher is a one-run choice, never a saved default.

Ctrl+V work runs in the same executable outside Explorer, with tray
notifications and no console. Starting it detaches the resident from the
launching terminal's process job and standard streams, so closing that terminal
does not stop Smart Paste. If the host forbids independent processes, startup
reports an error instead of starting a listener that will disappear with it;
start Smart Paste from the Explorer launcher in that case. Login startup remains
a separate opt-in. The right-click JustPaste action retains the
normal console result and uses `--clipboard --saved-settings`. Explorer's own
mouse **Paste** command is unchanged; use **JustPaste** for filtered mouse
pasting. Only ordinary File Explorer windows are hooked, not Desktop or
application file dialogs. No administrator service or extra executable is used.

A paste that takes longer than about a second opens a small progress window
without taking keyboard focus. It shows the destination, current step, elapsed
time, and download percentage, speed, and time remaining when available.
Tool downloads and update checks appear as steps too. Percentage describes the
current stream; a video may download audio separately before combining them.
**Cancel paste** stops the operation and keeps completed files. Closing the
window hides progress while the paste continues. Successful pastes close the
window automatically; errors stay visible until dismissed.

`--status` also shows pause/busy state, the last Explorer paste decision, and
the last notification, so an error remains readable after its balloon disappears.
That notification is kept only in memory and may include a saved filename.
`--diagnose` checks the current focus and each open shell view without displaying
clipboard contents, URLs, or folder paths. Explorer calls run on a dedicated
STA message-loop thread; UI Automation and file work remain outside it.
The keyboard hook has its own message thread, separate from tray menus and
settings reads. Key-repeat suppression stays within the original Explorer file
control; Alt+V and paste in other apps pass through.

## Clipboard images and text

Windows supports both `PNG` and `image/png` clipboard formats and DIB/DIBV5 images, including
screenshots. Output is `Image YYYY-MM-DD HH-MM-SS.png`, with original dimensions
and available transparency retained. A collision gets a numbered name. Images
are decoded with dimension/allocation limits and published atomically. Valid
clipboard PNGs keep their original bytes, including color metadata and alpha;
bitmap clipboard data is converted to PNG. Pasting never replaces or clears
the clipboard.
Plain text is opt-in (`--text`) and becomes `Text YYYY-MM-DD HH-MM-SS.txt` in
UTF-8. Image/file clipboard support is currently Windows-only; other platforms
retain text/link clipboard readers.

## Optional copy filters

Filters are **off by default**. With filters off, Explorer keeps handling normal
file Ctrl+V. Enable either tray filter or set custom ignored names to let
JustPaste copy the selected files/folders while omitting unwanted contents.
Cut/move and virtual file transfers remain with Explorer regardless of filters.
Explicit `justpaste --clipboard` refuses a cut payload, never deletes its source.

```powershell
justpaste --copy project -o existing-folder --ignore-node-modules --dry-run
justpaste --copy project -o existing-folder --exclude 'node_modules;.cache;build/temp' --respect-gitignore --dry-run
justpaste --clipboard --saved-settings -o existing-folder --dry-run
```

Remove `--dry-run` after reviewing the mappings. `--copy PATH` is repeatable;
its destination must already exist. Exclusions are semicolon-separated exact
names (any depth) or paths relative to each selected folder, case-insensitive;
wildcards and `..` are rejected. Git rules honor nested `.gitignore`, negation,
and `.git/info/exclude`, including in folders without a Git repository. Global
Git excludes are not loaded. These are ignore-rule filters, not Git tracked-file
selection: even explicitly selected ignored files/folders are skipped.

Sources are preserved. Copies keep empty folders, skip nested symlinks/junctions,
and refuse selected links, self/descendant destinations, or ambiguous selected
names before writing. Existing output roots receive numbered names; files are
published atomically and never overwritten. Cancellation/errors keep completed
files and directories, with no incomplete file published under its final name.
This is a content copy, not a filesystem backup: ACLs, alternate streams, and
timestamps are not promised.

The bare launcher saves filter settings atomically. Explicit headless calls use
only their supplied flags unless `--saved-settings` is passed. Saved output
folders and playlist choices never redirect Explorer Smart Paste. A playlist
download remains an explicit CLI/launcher choice.

## How a link is handled

1. **A repository link goes to Git.** Repository homepages on GitHub, GitLab
   (including subgroup paths), Bitbucket, Codeberg, Gitea, SourceHut, and Azure
   Repos are recognized, as are HTTP(S) clone links ending in `.git`, `ssh://`,
   `git://`, and scp-style SSH links such as `git@host:owner/repo.git`.
   File, issue, release, and archive pages keep the download path.
2. **Other links are requested.** If the answer is a file of any kind (image,
   video, audio, PDF, archive, anything that is not a web page), it is saved
   as-is. Nothing is converted.
3. **A web page goes to yt-dlp**, which supports YouTube, TikTok, Instagram,
   X, Reddit, Vimeo, and most other video and social sites, plus a generic
   extractor for pages that simply embed a video. A link the plain request
   could not fetch is tried through yt-dlp as well.
4. **If yt-dlp finds nothing**, the video or image the page declares for link
   previews (`og:video`, `og:image`, and the Twitter equivalents) is saved.
5. Otherwise the link is reported as failed and the run exits with status 1.
   Other links in the same run are still processed.

Git creates a normal checkout of the default branch with history, remote
branches, tags, and `origin`. Copy exclusions do not filter the checkout.
Submodules follow ordinary `git clone` behavior and are not initialized
automatically. URL query strings and fragments are dropped from repository
homepage links; a file or branch URL does not select a branch for cloning.

yt-dlp downloads the highest resolution available, prefers H.264 and MP4 at
that resolution so the result plays everywhere, and merges into an `.mp4`. A
playlist link downloads one entry unless `--playlist` is passed. To shrink or
convert the result, run `justvideo`, `justmp3`, or another tool on it, or give
those tools the link directly.

## Names and safety

| Source | Saved name |
| --- | --- |
| Direct file | The server's `Content-Disposition` name, else the last part of the address, else the host; an extension is added from the content type when the name has none |
| yt-dlp | `<title>.<ext>`, or `<id>.<ext>` when the page has no usable title |
| Git repository | A subfolder named after the repository, without `.git`; taken names become `repo (2)`, `repo (3)`, including when the repository name contains dots |

Every name is tidied the same way before it is saved, so a caption-length
TikTok title and a `Content-Disposition` header full of symbols both come out
as a short plain name:

- Letters and digits of any script are kept, with spaces and `- _ . , ' ( ) & +`.
  Emoji, symbols, quotes, and path characters become a space.
- Links are dropped. Hashtags are dropped too, unless they are the whole title.
- A name longer than 80 characters is cut at a word.
- `POV: you finally did it 😭😭 #fyp #gym` is saved as `POV you finally did it.mp4`.

yt-dlp itself downloads under a fixed name (`001.mp4`) inside the hidden
folder and the title is applied afterwards, so a title yt-dlp or FFmpeg cannot
write, or one that pushes a path past the Windows limit, no longer fails the
download.


- Nothing is overwritten. A taken name is saved as `name (2).ext`, `name (3).ext`.
- A download is written under a hidden `.justpaste-*` name inside the
  destination and renamed only when complete, so an interrupted run leaves
  nothing behind under a real name.
- A Windows device name such as `con` or `com1` is saved with a leading `_`.
- `--dry-run` only lists the links and the destination; it makes no request.
- Clones are staged beside their destination and published atomically under
  an unused folder name. Failed or cancelled clones remove their staging
  folder and leave existing files and repositories intact. Clone errors are
  reported directly; they do not fall through to yt-dlp.

## Inputs

Links can be given as arguments, piped on standard input, or read from the
clipboard with `--clipboard`. Text around a link is ignored, so a copied
sentence or a list works; a bare `www.` address is treated as HTTPS. Explicit URL
arguments must contain HTTP(S) links or a supported Git repository link;
Windows clipboard images and files are also accepted, and ordinary text
requires `--text`.

Clipboard readers preserve the clipboard while they take a bounded snapshot:

| Platform | Reader | Clipboard formats |
| --- | --- | --- |
| Windows | Native Windows API | PNG/DIB images, copied files, text and links |
| macOS | Native pasteboard through the system `osascript` command | PNG/TIFF/JPEG images, copied local files, text and links |
| Linux Wayland | `wl-paste` from `wl-clipboard` | Images, GNOME/KDE copied local files, text and links |
| Linux X11 | `xclip`; `xsel` is a text fallback | Images, copied local files, text and links with `xclip` |

Linux clipboard helpers are not installed automatically. If a reader or
desktop session is missing, JustPaste explains how to install the helper and
how to use CLI arguments or stdin instead. Clipboard images become PNG with
alpha preserved; an original PNG keeps its bytes. Local file URLs are decoded
without losing spaces or Unicode. Cut/move payloads remain the file manager's
job; JustPaste refuses them and retains the sources.

## Dependencies and privacy

Direct file links need nothing. Page links need yt-dlp and FFmpeg, which are
downloaded automatically when they are not on `PATH` or in the managed tools
folder. Successful yt-dlp update checks are cached for twelve hours; failures
retry after fifteen minutes. Smart Paste checks periodically while running,
including when no paste is active. Both behaviors are described in the
[downloads guide](downloads.md). A link is fetched from the address you give
it, so the site sees the request. Only download what you have the right to.

Repository links need Git, resolved from `PATH` or an explicit `GIT_BIN`.
Missing Git requires interactive installation confirmation; it is never
downloaded automatically. A missing Git executable produces a warning with
installation guidance; an invalid `GIT_BIN` warns how to fix the override.
Headless runs fail without installing Git. Private repositories use your existing Git credentials
or SSH setup. Unattended clones do not open sign-in or SSH password prompts.
Repository-only CLI pastes do not resolve or update yt-dlp or FFmpeg.

Bare `justpaste` opens the launcher. A blank **Links** row pastes the
clipboard; **Output folder** and **Download playlists** are saved, and the
links themselves never are.
