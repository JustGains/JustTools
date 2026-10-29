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
justpaste https://x.com/Cloudflare                   # public profile photo
justpaste https://www.facebook.com/cloudflare        # public profile photo
justpaste --markdown https://example.com             # rendered page as .md
justpaste https://github.com/owner/repo             # clones into ./repo
justpaste git@github.com:owner/repo.git -o projects # clones into projects/repo
justpaste https://www.youtube.com/watch?v=ID       # saves "<title>.mp4"
justpaste https://www.tiktok.com/@user/video/ID    # same, through yt-dlp
justpaste --clipboard                              # every link in the copied text
justpaste -o downloads https://a.test/x.pdf https://b.test/clip
justpaste --dry-run --clipboard                    # list the links; no network
```

```sh
justpaste --clipboard --as md                      # rich text as Markdown
justpaste --clipboard --as ocr                     # text from a screenshot (Windows)
justpaste --clipboard --ai "Summarize it" --ai-service anthropic
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

## Paste as (Advanced Paste)

JustPaste covers the options of PowerToys Advanced Paste, aimed at folders: the
clipboard always becomes a new file, and nothing is typed into other apps.

| Format | Takes | Saves |
| --- | --- | --- |
| `txt` | Text or rich text | `Text <date time>.txt`, UTF-8 |
| `md` | Text or rich text | Markdown; copied HTML keeps headings, lists, links, tables, and emphasis |
| `html` | Text or rich text | A standalone UTF-8 page holding the copied fragment (plain text is escaped) |
| `json` | Text | Pretty JSON from JSON, XML, CSV/TSV with a header row, INI, or `key: value` lines; other text becomes a JSON string |
| `png`, `jpg`, `webp` | An image (or one image file with `--copy`) | `Image <date time>.<ext>`; PNG and WebP keep alpha, JPEG is flattened onto white |
| `ocr` | An image | `Image text <date time>.txt`, read locally by the Windows OCR engine (Windows only) |
| `mp3`, `mp4` | Copied audio/video files, `--copy` media, or links | Transcoded with JustMP3/JustVideo in a hidden workspace; sources are kept |

```sh
justpaste --clipboard --as md                  # copied web page selection -> .md
printf 'name,count\nApples,3\n' | justpaste --as json
justpaste --clipboard --as ocr                 # screenshot text -> .txt (Windows)
justpaste --as jpg --copy screenshot.png -o exports
justpaste --as mp3 --copy talk.mov             # keeps talk.mov
justpaste --as mp4 https://www.youtube.com/watch?v=ID
```

Every result is published atomically under an unused name, and `--dry-run`
writes nothing. A format that does not fit the clipboard fails before anything
is written; `--as` cannot be combined with `--markdown`, `--profile-photo`, or
`--playlist`.

### AI paste

AI paste is off until it is enabled in settings or requested with `--ai`. It
sends only the copied (or piped) text and the instruction to the configured
service and saves the answer as a new file, `.txt` unless `--as md`, `html`, or
`json` picks the extension.

```sh
justpaste --clipboard --ai "Summarize in three bullets" --ai-service anthropic
justpaste --clipboard --ai-action fix-spelling --ai-service openai --ai-model YOUR_MODEL
justpaste --clipboard --ai-action Summarize --saved-settings   # a saved custom action
printf 'notes' | justpaste --ai "Translate to French" --ai-service ollama --ai-model llama3.2
```

| Service (`--ai-service`) | Endpoint | Key from the environment |
| --- | --- | --- |
| `openai` | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| `azure` | `--ai-endpoint https://NAME.openai.azure.com`, `--ai-deployment`, optional `--ai-api-version` (default `2024-10-21`) | `AZURE_OPENAI_API_KEY` |
| `mistral` | `https://api.mistral.ai/v1` | `MISTRAL_API_KEY` |
| `gemini` | Google's OpenAI-compatible endpoint | `GEMINI_API_KEY` |
| `ollama` | `http://localhost:11434/v1` | none |
| `foundry` | Foundry Local's URL, such as `http://localhost:5273/v1` | none |
| `compatible` | Any OpenAI-compatible `/v1` URL (LM Studio, vLLM, a gateway) | optional |
| `anthropic` | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |

`JUSTPASTE_AI_API_KEY` is a fallback key for every service. `--ai-endpoint`
overrides any endpoint. Anthropic defaults to `claude-opus-5-5` and lets the
API retry a declined request on a model it chooses; every other service needs
`--ai-model` (Azure can use the deployment name instead). OpenAI requests are
first checked with OpenAI's moderation endpoint, as in PowerToys; flagged text
is not transformed, and `--no-ai-moderation` skips the check.
`--ai-system-prompt` adds standing instructions. Keys are read like the
Cloudflare token: from the process environment or, on Windows, the stored user
environment. They stay in memory and never appear in defaults, errors, or the
Headless footer. A model's answer is saved as given apart from one wrapping
code fence; review it before relying on it.

**Fix spelling and grammar** is built in (`--ai-action fix-spelling`). Custom
actions are saved one per line as `Name: instruction` (up to 20) in settings;
the CLI runs them by name with `--saved-settings`.

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

With Explorer's filesystem file pane focused, **Ctrl+V** applies one saved
choice per kind of clipboard content:

| Clipboard | Ctrl+V choices (default first) |
| --- | --- |
| Text or rich text | Leave to Explorer, `.txt`, Markdown, HTML, JSON, or ask |
| Image | PNG, JPEG, WebP, text from image (OCR), or ask |
| Links | Download/clone/save the page, page as Markdown (Cloudflare), MP4, MP3, or ask |
| Audio and video files | Copy normally, transcode to MP4 or MP3, or ask |

The older **Save plain text as .txt** setting still selects `.txt` until a
text choice is saved. Copied files that are not all media keep the copy-filter
rules below. "Ask" opens the picker.

### The paste-as picker

Instead of one shortcut per format, JustPaste has a single picker shortcut,
**Ctrl+Shift+V** by default (Win+Shift+V, Ctrl+Alt+V, or off in settings). In
Explorer's file pane it opens a menu at the cursor listing only the actions
that fit the clipboard: **Paste normally**, JustPaste's own download/copy
handling, Markdown pages, every paste-as format, transcoding, and, with AI
enabled, Fix spelling and grammar, each custom action, and **Paste with AI…**.
Entries are numbered 1–9 and 0 for the keyboard, the Ctrl+V choice is bold,
and Esc closes the menu without pasting. A header line describes the clipboard
(image size, link count, or the first words of text) unless the clipboard
preview is turned off; it never shows paths.

AI actions open a review window when **Review answers before saving** is on
(PowerToys' custom preview): the instruction, the answer to read or edit, and
Save. **Paste with AI…** always opens it so an instruction can be typed. The
window can close when it loses focus. With review off, a custom action saves
its answer directly with the progress window.

By default, Ctrl+V saves a clipboard image as PNG or downloads copied links
into the active tab's folder. Actual image data takes priority over a browser's
accompanying URL. Address-bar,
search, rename, other-app, and unsupported/virtual-folder pastes stay native.
Ctrl+V is only taken when the clipboard holds an image, text, or (with copy
filters on) copied files. Anything else, such as normal file copies with
filters off, Outlook attachments, or files dragged out of a zip or phone, goes
straight to Explorer with no delay. The destination and clipboard are checked at activation; switching folders
during a download does not redirect its output. There is no clipboard history
or copy-time downloading.

Click the violet-and-blue tray icon to open **JustPaste settings**, or run
`just paste --settings`. The native window has three tabs: **Paste** (the
Ctrl+V choices, the picker shortcut, clipboard preview, which picker actions
are offered, and sign-in startup), **AI** (enable, service, model, endpoint,
Azure deployment and API version, system prompt, OpenAI moderation, review
before saving, close on focus loss, Fix spelling, Paste with AI…, and custom
actions), and **Copy filters**. **Save changes** validates paths, custom
actions, and the AI configuration and saves everything together; **Cancel**
leaves it unchanged. No API key is ever entered or saved there. Saved fields
take effect without restarting Smart Paste. Settings also opens when the
resident is stopped and does not need a console.
Opening settings from the tray does not request administrator rights or process
breakaway, so it also works when the resident is inside a restricted Windows
process job. The background resident's startup detachment remains separate.

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
normal console result and uses `--clipboard --saved-settings`, so it follows
the same saved Ctrl+V choices ("ask" falls back to the default handling). Explorer's own
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

`--status` shows whether copy filters are on (JustPaste copies files) or off
(Explorer copies files), and also pause/busy state, the last Explorer paste decision, and
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
Files are copied with the system copy (CopyFileEx on Windows, which keeps
modified times, attributes, and alternate streams and uses block cloning or
server-side copies where the volume offers them), several files at a time.
Elsewhere the platform copy keeps modified times. ACLs are not copied; this is
not a filesystem backup.

The bare launcher saves filter settings atomically. Explicit headless calls use
only their supplied flags unless `--saved-settings` is passed. Saved output
folders and playlist choices never redirect Explorer Smart Paste. A playlist
download remains an explicit CLI/launcher choice.

## How a link is handled

Public X/Twitter, Facebook, Instagram, and TikTok profile URLs save the profile
photo before the media downloader is considered. X profile banners are ignored;
the original avatar is preferred, with the published thumbnail as a fallback.
Filenames include the site and account, such as `x-Cloudflare-profile.jpg`.
Image bytes and alpha are retained, and taken names get a number. Post, video,
and reel URLs keep their existing download behavior. Private/login-only profile
photos are not retrieved. `--profile-photo` explicitly chooses this route.

When `CLOUDFLARE_API_KEY` or `CLOUDFLARE_API_TOKEN` is available, ordinary HTML
pages become Markdown through [Cloudflare Browser Run](https://developers.cloudflare.com/browser-run/quick-actions/markdown-endpoint/).
Direct files, Git repositories, social profile photos, and video pages keep
their own routes. `--markdown` forces Markdown for a page, including a media or
profile page. It cannot be combined with `--profile-photo`.

The token needs **Browser Rendering - Edit** permission. Set
`CLOUDFLARE_ACCOUNT_ID` to select the account, or let JustPaste discover a single
account through account/zone read access. Several accounts or missing discovery
permissions require an explicit account ID. `CLOUDFLARE_API_KEY` can also be a
global API key when `CLOUDFLARE_EMAIL` is supplied; `CLOUDFLARE_API_TOKEN` always
uses bearer authentication.

Windows user environment values are read when the launching process has not
inherited them yet. An explicitly empty process value overrides that stored
value. Credentials remain in memory, are excluded from errors, and never become
launcher defaults. Cloudflare receives the page URL to render it; the token is
sent only to Cloudflare. The bare launcher's Markdown and Profile photo rows
apply to one run. Markdown files use the page title or Markdown title, are
published atomically, and never overwrite existing files. Empty/error responses
save nothing. A profile may use Cloudflare's rendered HTML as a fallback when
public static metadata is absent.

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
clipboard; **Output folder**, **Download playlists**, and the AI service,
model, and endpoint are saved. The links, **Paste as**, and the **AI
instruction** apply to one run and are never saved.
