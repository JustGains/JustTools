# JustPDF

`justpdf` (or `just pdf`) inspects, merges, splits, extracts, rotates, and
compresses PDFs, and saves their embedded images and hyperlinks. PDF inputs are never modified.
Run it bare for the console UI: its **Operation** row offers every command
below, and the footer shows the exact headless command.

| Command | Default output |
| --- | --- |
| `justpdf guide.pdf` | Page count, sizes, and metadata in the terminal |
| `justpdf merge a.pdf b.pdf` | `merged.pdf` in the current folder |
| `justpdf split guide.pdf` | `guide-pages/001.pdf`, `002.pdf`, … |
| `justpdf extract --pages 1-3,last guide.pdf` | `guide-pages-1-3,last.pdf` |
| `justpdf rotate --degrees 180 guide.pdf` | `guide-rotated.pdf` |
| `justpdf images guide.pdf` | `guide-images/p001-01.jpg`, `p003-01.png`, … |
| `justpdf links guide.pdf` | `guide-links.txt` |
| `justpdf compress guide.pdf` | `guide-compressed.pdf` |

All outputs are written beside the PDF unless `--output` names another path.
Page ranges are one-based, such as `1-3,5,last`. `--dry-run` previews the plan
without writing, and existing outputs are replaced only after confirmation or
`--yes`. Merge and Compress accept several PDFs; every other operation takes
exactly one.

## Compress

`compress` writes a smaller copy of each PDF. `--level` picks how hard it works:

| Level | Images | Typical use |
| --- | --- | --- |
| `balanced` (default) | At most 150 ppi, JPEG quality 75 | Sharing and reading on screen |
| `smallest` | At most 96 ppi, JPEG quality 50 | Email attachments and upload limits |
| `lossless` | Unchanged, byte for byte | Archival copies that must render identically |

Every level drops unused objects and earlier saved revisions, recompresses
streams at the best Flate setting, and packs objects into compressed object
streams. Text, vector artwork, fonts, links, bookmarks, and form fields are
kept.

The lossy levels re-encode 8-bit gray and RGB images, whether stored as JPEG or
raw pixels, as JPEG. An image is downsampled only when it has more pixels than
the largest page needs at the level's resolution, so an image drawn smaller
than the page can keep more than that. A re-encoded image is kept only when it
saves at least 10%, so flat graphics that compress better losslessly stay as
they were. Images that are CMYK, indexed, 1-bit, 16-bit, JPEG 2000, CCITT, or
JBIG2, use color-key masks or `/Decode` arrays, or serve as transparency masks
are left unchanged. Inline images are not rewritten.

The copy is written only when it is smaller than the source; otherwise the
command reports that the PDF is already compact and writes nothing. With one
PDF, `--output` names the file, or an existing folder that receives
`<name>-compressed.pdf`; with several, it is the folder. A digitally signed PDF
is compressed with a warning, because any rewrite breaks its signature.

```sh
justpdf compress guide.pdf --dry-run
justpdf compress guide.pdf --level smallest
justpdf compress scans --recursive --level lossless --output compressed
```

## Images

`images` saves every image object that the selected pages draw, including
images inside forms, tiling patterns, and annotation appearances. Each image is
saved once, named after the first page that uses it and its order on that page:
`p003-02.png` is the second image found on page 3.

- A JPEG is copied byte-for-byte when it has no transparency mask, or when its
  mask is fully opaque, so no quality is lost.
- A JPEG with real transparency becomes an RGBA PNG: the decoded photo plus the
  PDF's exact mask as alpha, carrying the color profile. Fully transparent
  pixels are stored as transparent black, which keeps cut-outs compact.
- JPEG 2000 is copied as `.jp2`; a transparency mask cannot be applied, and the
  command says so.
- Uncompressed and Flate/LZW/ASCII85 images (gray, RGB, CMYK, ICC, and indexed
  color at 1–16 bits, plus stencil and color-key masks) become PNG. CMYK is
  converted to RGB without color management.
- CCITT, JBIG2, RunLength, and spot-color (Separation, DeviceN, Lab) images are
  reported and skipped. Inline images embedded directly in page content are not
  exported, and vector artwork and gradients are not images.

`--output` is the folder that receives the files. Unsupported images are
warnings; an image that fails to decode is reported, the rest are still saved,
and the command then exits nonzero.

## Links

`links` writes each unique URL once, one per line, in the order it first
appears. It reads clickable links on the selected pages, including chained
link actions, and bookmark URLs when the whole document is selected. Internal
page jumps are skipped, and plain text that is not a clickable link is not
detected.

URLs are trimmed and matched without casing differences. YouTube variants use
one clean watch URL per video, following the [JustLinks rules](links.md).
The file is UTF-8 with a final
newline. `--output` accepts a file path, or an existing folder that receives
`<name>-links.txt`. When a PDF has no links, nothing is written.

For link titles, URLs typed as page text, Office and text files, or whole
folders, use [JustLinks](links.md): `justlinks guide.pdf --csv`.

```sh
justpdf images guide.pdf --dry-run
justpdf images guide.pdf --output images
justpdf links guide.pdf --output links_output.txt
```

## Console UI

The launcher saves the operation, output, page range, rotation, compression
level, and recursion. The page range applies to Extract, Rotate, Images, and
Links, rotation only to Rotate, and the compression level only to Compress;
saved values an operation does not use are left out of its Headless command. The details panel shows the resolved destination.
