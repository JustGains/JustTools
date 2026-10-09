# JustZip

Archive files, folders, or a mixed selection into one ZIP. Ordinary folders
need no external tools; Git working trees use Git to honor their ignore rules.

```sh
justzip                                  # console launcher
justzip photos --dry-run                  # preview contents and destination
justzip photos --compression balanced
justzip notes.txt assets --output bundle.zip --dry-run
justzip notes.txt assets --output bundle.zip
justzip -- -draft.txt                     # option-like input name
```

Explicit arguments and redirected input use deterministic headless behavior.
With no input path in headless mode, the current folder is archived.

## Inputs and output names

| Input | ZIP contents | Default output in the current directory |
| --- | --- | --- |
| One folder `photos` | Its contents at the archive root | `photos.zip` |
| One file `notes.txt` | `notes.txt` | `notes.txt.zip` |
| Several files/folders | Each selected file or folder under its own name | `archive.zip` |

`--output` accepts an exact file path or an existing destination directory.
A path ending in a slash is also treated as a directory. Ordinary folders
include hidden files and empty directories recursively. Git metadata (`.git`)
is excluded during traversal. Repeated identical inputs are deduplicated.

Sources are kept. Existing destination ZIPs are replaced atomically only after
the new archive has been fully written. An output inside a selected folder
is excluded from its own contents. An output that equals an explicitly selected
source is rejected. Conflicting archive names, including case-only duplicates
and a file shadowing another entry's directory, fail before anything is written.
Choose another output path or archive conflicting selections separately.

Symlinks are stored as links without following them. Absolute targets and
targets escaping their selected folder are rejected. Invalid or unreadable
inputs fail instead of silently creating a partial archive.

## Git folders

For a Git working tree, a linked worktree, or a subfolder of either, Git supplies
tracked and unignored working-tree files. Nested `.gitignore`, `.git/info/exclude`,
and global ignore rules all apply. Explicitly selected files are included even
if Git ignores them. Nested repositories inside ordinary folders also use their
own Git file lists. Gitlinks (submodules) and missing tracked files are skipped
and reported, as in the original repository archiver.

Git is resolved only when needed. If Git cannot read a working tree, the command
fails; it does not fall back to including ignored files.

## Compression, preview, and launcher

`--compression` accepts `fast`, `balanced`, or `smallest` (the default).
`--dry-run` reports the destination, entry count, compression, and up to 100
entry names without creating folders or archives.

The launcher accepts multiple inputs separated by semicolons. Output and
compression are saved defaults; input paths and dry-run choices are never saved.
The footer shows the exact **Headless:** command and the destination and overwrite
policy are visible above it.

## Windows Explorer

After `just context install`, any file, folder, mixed selection, or folder
background offers **JustZip** presets, **preview included files**, and
**more options…** in the JustTools flyout. Presets write beside the first
selected item. A folder background acts on that folder, so its ZIP is beside
the folder. More options starts from the same directory with all inputs filled
in. See the [context menu guide](context-menu.md).
