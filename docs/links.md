# JustLinks

`justlinks` (or `just links`) extracts every unique link from PDFs, Excel,
Word, and PowerPoint files, and text files, and writes each link once in the
order it first appears. Inputs are only read. Run it bare for the console UI;
the footer shows the exact headless command.

```sh
justlinks guide.pdf plan.xlsx                  # links.txt in the current folder
justlinks guide.pdf plan.xlsx --csv            # links.csv with titles
justlinks programs --recursive --csv -o all.csv
justlinks notes.md -o -                        # print instead of writing
pbpaste | justlinks                            # piped text prints its links
```

## Output

A text list has one URL per line. `--csv`, or an output path ending in
`.csv`, writes one row per unique URL:

| Column | Meaning |
| --- | --- |
| `url` | The first spelling of the link; YouTube videos use a clean canonical watch URL |
| `title` | The link's text; see below |
| `file` | Where the URL first appeared |
| `location` | `page 26`, `'Week 1'!C5`, `paragraph 3`, `slide 2`, `line 14`, or `bookmarks` |
| `occurrences` | How many times it appears across all inputs |

The CSV is UTF-8 with a byte-order mark so spreadsheet apps read it correctly.
Fields that start with `=`, `+`, `-`, or `@` are prefixed with `'` so a
spreadsheet cannot run them as formulas.

`--output` also accepts an existing folder, which receives `links.txt` or
`links.csv`, and `-` for standard output. An existing output is replaced only
after confirmation or `--yes`. When the output lies inside a scanned folder,
it is never read back in as an input.

## Titles

A URL's title is the most common text it appears with, so one imprecise
occurrence cannot outvote the rest.

- **PDF:** the text drawn under the link. A link drawn as one rectangle per
  line of wrapped text is treated as a single link. Bookmarks use their titles.
- **Excel:** the cell's text; a `HYPERLINK` formula's friendly name; or, for a
  bare URL, the first text cell in its row, such as the exercise name.
- **Word and PowerPoint:** the hyperlink's own text, or the label before it in
  its paragraph. Word HYPERLINK fields use their displayed text.
- **Typed URLs everywhere:** the words before the URL on its line, without
  separators, bullets, or list numbers. `OMNI-GRIP LAT PULLDOWN:
  https://youtu.be/NDmJNX9JrLs?t=4m7s` is titled `OMNI-GRIP LAT PULLDOWN`, and
  `- [Bench](https://…)` is titled `Bench`. With nothing before the URL, the
  words after it are used. HTML anchors use their visible text.
- **CSV and TSV files:** each field is read separately; a bare URL takes its
  row's first text field.

A title is empty when a link has no nearby text, such as a linked image.

## What is read

| Source | Links found |
| --- | --- |
| PDF | Link annotations (including chained actions), URLs typed in page text, bookmarks |
| `.xlsx` / `.xlsm` | Cell hyperlinks, HYPERLINK formulas, URLs typed in cells and comments |
| `.docx` / `.docm` | Hyperlinks, HYPERLINK fields, URLs typed in paragraphs, headers, and notes |
| `.pptx` / `.pptm` | Text and shape hyperlinks, URLs typed on slides and notes |
| Text files | `http`, `https`, `ftp`, and `mailto` URLs and `www.` addresses |

Files named explicitly are recognized by content, so a misnamed file still
works. Folders include `.pdf`, the Office formats above, and `.txt`, `.md`,
`.csv`, `.tsv`, `.html`, `.xml`, `.json`, `.yaml`, `.log`, and `.rtf`; add
`--recursive` for nested folders. Legacy `.xls`, `.doc`, and `.ppt` files,
password-protected Office files, and other binary files are reported as
skipped. A file that cannot be read is reported, the rest are still written,
and the command then exits nonzero.

Links are deduplicated case-insensitively across the entire URL after trimming
surrounding whitespace. The first spelling is kept. Non-YouTube parameters,
fragments, and trailing slashes otherwise remain significant.

YouTube video links are written as `https://www.youtube.com/watch?v=VIDEO_ID`.
`youtu.be`, watch, Shorts, live, embed, legacy `/v/`, `/e/`, and `/watch/v/`
links all merge by video ID, including mobile, music, gaming, no-cookie, and
`youtube.googleapis.com` hosts. All extra parameters and fragments are removed,
including timestamps, share tokens, tracking, playlist IDs, and playlist indices.
Percent-encoded IDs and destinations in YouTube attribution, redirect, and
oEmbed links are recognized, as are old hash-based watch URLs. Bare YouTube
hosts such as `youtu.be/ID` are also found in text.

Video IDs match case-insensitively too, retaining the first ID's spelling in
the output. This deliberately merges IDs differing only by casing; the tool
does not verify which spelling plays. Counts and title selection combine every
matched variant, while file and location still refer to the first occurrence.
Playlist-only links, channels, clips, malformed video URLs, and unknown
shorteners are kept as ordinary links: destinations requiring a network lookup
are not resolved. These rules also apply to `justpdf links`.

A URL wrapped across two lines of PDF text is
kept whole when a link annotation covers it; otherwise only its first line
can be recognized.

## Large batches

Files are read in parallel: by default one job per CPU, up to 8. Use
`--jobs N` to change this. Results are merged in input order, so the output is
the same for any job count. Progress lines name each file that has links; the
summary counts files without links and skipped files.
