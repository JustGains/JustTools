//! `justlinks`: every unique link in PDFs, Office documents, and text files.

mod normalize;
mod office;
mod text;

pub(crate) use text::{labeled_urls, title_from};

use crate::common::{
    absolute_lexical, atomic_write, collect_inputs, confirm_replacement, display_path, parse_cli,
    read_stdin, same_path, stdin_is_terminal,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Display;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

/// Extensions read from folders; explicitly named files are detected by
/// content. Legacy Office formats are listed so they are reported as skipped.
const EXTENSIONS: &[&str] = &[
    "pdf", "xlsx", "xlsm", "docx", "docm", "pptx", "pptm", "txt", "md", "markdown", "csv", "tsv",
    "html", "htm", "xml", "json", "yaml", "yml", "log", "rtf", "xls", "doc", "ppt",
];
/// Largest text file scanned.
const MAX_TEXT_BYTES: u64 = 256 << 20;
/// Default parallelism ceiling; PDFs are held in memory while read.
const MAX_DEFAULT_JOBS: usize = 8;

#[derive(Debug, Parser)]
#[command(
    name = "justlinks",
    about = "Extract every unique link from PDFs, Office documents, and text files.",
    after_help = "Reads PDF links, bookmarks, and URLs in page text; Excel, Word, and PowerPoint\nhyperlinks, HYPERLINK formulas, and URLs typed in cells or text; and URLs in text,\nMarkdown, CSV, HTML, and JSON files. Each link is written once, in first-seen order.\n--csv adds each link's title (its link text, cell, or label such as `Squat: <url>`),\nfile, location, and occurrence count. Inputs are never changed.\nWith no files, piped text is scanned and its links are printed.\nThe installed JustTools multicall alias opens a saved-defaults launcher when run bare."
)]
struct Cli {
    /// Output file, an existing folder, or - for standard output
    /// (default: links.txt, or links.csv, in the current folder).
    #[arg(short = 'o', long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Write CSV with each link's title, file, location, and count
    /// (implied by a .csv output).
    #[arg(long)]
    csv: bool,

    /// Include nested folders.
    #[arg(short = 'r', long)]
    recursive: bool,

    /// Files read at once (default: CPU count, up to 8).
    #[arg(short = 'j', long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..=256))]
    jobs: Option<u16>,

    /// Replace an existing output without asking.
    #[arg(short = 'y', long)]
    yes: bool,

    /// Show link counts and the destination without writing.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Files or folders to read.
    #[arg(value_name = "FILE")]
    inputs: Vec<PathBuf>,
}

/// One link occurrence, its title (possibly empty), and where it appeared.
pub(crate) struct Found {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) location: String,
}

/// A link without surrounding whitespace, NULs, or byte-order marks.
fn clean(link: &str) -> Option<&str> {
    Some(link.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\0' | '\u{feff}')
    }))
    .filter(|link| !link.is_empty())
}

/// Unique links in first-seen order, counting every occurrence.
#[derive(Default)]
pub(crate) struct LinkSet {
    seen: HashSet<String>,
    unique: Vec<String>,
    found: usize,
}

impl LinkSet {
    pub(crate) fn add(&mut self, link: &str) {
        let Some(link) = clean(link) else {
            return;
        };
        self.found += 1;
        let (key, url) = normalize::link(link);
        if self.seen.insert(key) {
            self.unique.push(url);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.unique.is_empty()
    }

    pub(crate) fn summary(&self) -> String {
        summary(self.unique.len(), self.found)
    }

    /// One link per line with a final newline.
    pub(crate) fn text(&self) -> String {
        lines(self.unique.iter().map(String::as_str))
    }
}

fn summary(unique: usize, found: usize) -> String {
    format!("{unique} unique link(s) from {found} found")
}

fn lines<'a>(links: impl Iterator<Item = &'a str>) -> String {
    links.flat_map(|link| [link, "\n"]).collect()
}

/// Unique links with where each first appeared, its titles, and its count.
#[derive(Default)]
struct Table {
    rows: Vec<Row>,
    index: HashMap<String, usize>,
    found: usize,
}

struct Row {
    url: String,
    file: String,
    location: String,
    /// Distinct titles in first-seen order, with how often each occurred.
    titles: Vec<(String, usize)>,
    count: usize,
}

impl Row {
    /// The most frequent title, which outvotes link rectangles that clip
    /// their text; ties go to the earliest.
    fn title(&self) -> &str {
        let mut best: Option<&(String, usize)> = None;
        for candidate in &self.titles {
            if best.is_none_or(|best| candidate.1 > best.1) {
                best = Some(candidate);
            }
        }
        best.map_or("", |(title, _)| title)
    }
}

impl Table {
    fn add(&mut self, item: &Found, file: &str) {
        let Some(url) = clean(&item.url) else {
            return;
        };
        self.found += 1;
        let (key, url) = normalize::link(url);
        let index = *self.index.entry(key).or_insert_with(|| {
            self.rows.push(Row {
                url,
                file: file.to_owned(),
                location: item.location.clone(),
                titles: Vec::new(),
                count: 0,
            });
            self.rows.len() - 1
        });
        let row = &mut self.rows[index];
        row.count += 1;
        let title = item.title.trim();
        if !title.is_empty() {
            match row.titles.iter_mut().find(|(known, _)| known == title) {
                Some((_, count)) => *count += 1,
                None => row.titles.push((title.to_owned(), 1)),
            }
        }
    }

    fn text(&self) -> String {
        lines(self.rows.iter().map(|row| row.url.as_str()))
    }

    fn csv(&self) -> String {
        let mut csv = String::from("url,title,file,location,occurrences\r\n");
        for row in &self.rows {
            let count = row.count.to_string();
            let fields = [
                row.url.as_str(),
                row.title(),
                &row.file,
                &row.location,
                &count,
            ];
            csv.push_str(&fields.map(csv_field).join(","));
            csv.push_str("\r\n");
        }
        csv
    }
}

/// Quotes a CSV field when needed, and prefixes `'` to text a spreadsheet
/// would otherwise run as a formula.
fn csv_field(value: &str) -> String {
    let value = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

pub fn run() -> Result<()> {
    let Some(options) = parse_cli::<Cli>()? else {
        return Ok(());
    };
    let csv = options.csv
        || options
            .output
            .as_deref()
            .and_then(extension)
            .is_some_and(|extension| extension == "csv");
    let default_name = if csv { "links.csv" } else { "links.txt" };

    if options.inputs.is_empty() {
        if stdin_is_terminal() {
            bail!("provide at least one file or folder, or pipe text");
        }
        let mut table = Table::default();
        for item in text_links(&read_stdin()?) {
            table.add(&item, "stdin");
        }
        // Piped text prints its links unless an output is named.
        let output = match options.output.as_deref() {
            Some(path) => destination(Some(path), default_name)?,
            None => None,
        };
        return finish(&table, output.as_deref(), &options, csv, "piped text", 0);
    }

    let output = destination(options.output.as_deref(), default_name)?;
    let to_stderr = output.is_none();
    let include = |path: &Path| extension(path).is_some_and(|value| EXTENSIONS.contains(&&*value));
    let collected = collect_inputs(&options.inputs, options.recursive, &include)?;
    for warning in &collected.warnings {
        eprintln!("justlinks: {warning}");
    }
    let mut files = collected.files;
    if let Some(output) = &output {
        if options
            .inputs
            .iter()
            .any(|input| absolute_lexical(input).is_ok_and(|input| same_path(&input, output)))
        {
            bail!("output cannot also be an input: {}", display_path(output));
        }
        // A previous result inside a scanned folder is never read back in.
        files.retain(|file| !same_path(file, output));
    }
    if files.is_empty() {
        bail!("no supported files found");
    }

    let jobs = options.jobs.map_or_else(
        || {
            thread::available_parallelism()
                .map_or(1, usize::from)
                .min(MAX_DEFAULT_JOBS)
        },
        usize::from,
    );
    let mut table = Table::default();
    let mut failed = 0;
    let mut skipped = 0;
    let mut without_links = 0;
    read_all(&files, jobs, |file, result| match result {
        Ok(found) => {
            let file = display_path(file);
            let links: Vec<&str> = found.iter().filter_map(|item| clean(&item.url)).collect();
            let unique: HashSet<String> = links.iter().map(|url| normalize::link(url).0).collect();
            if unique.is_empty() {
                without_links += 1;
            } else {
                note(
                    to_stderr,
                    format!("{file}: {}", summary(unique.len(), links.len())),
                );
            }
            for item in &found {
                table.add(item, &file);
            }
        }
        Err(error) => match error.downcast_ref::<Unsupported>() {
            Some(unsupported) => {
                skipped += 1;
                eprintln!("justlinks: {}: skipped: {unsupported}", display_path(file));
            }
            None => {
                failed += 1;
                eprintln!("justlinks: {error:#}");
            }
        },
    });
    let details: Vec<String> = [(without_links, "without links"), (skipped, "skipped")]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect();
    let mut source = format!("{} file(s)", files.len());
    if !details.is_empty() {
        source.push_str(&format!(" ({})", details.join(", ")));
    }
    finish(&table, output.as_deref(), &options, csv, &source, failed)
}

/// Reads files on `jobs` threads, handing results over in input order so
/// the output never depends on timing.
fn read_all(files: &[PathBuf], jobs: usize, mut handle: impl FnMut(&Path, Result<Vec<Found>>)) {
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, files.len()) {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(index) else {
                        break;
                    };
                    if sender.send((index, file_links(file))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut pending = BTreeMap::new();
        let mut expected = 0;
        for (index, result) in receiver {
            pending.insert(index, result);
            while let Some(result) = pending.remove(&expected) {
                handle(&files[expected], result);
                expected += 1;
            }
        }
    });
}

/// The output file, or `None` for standard output.
fn destination(output: Option<&Path>, default_name: &str) -> Result<Option<PathBuf>> {
    let path = match output {
        Some(path) if path.as_os_str() == "-" => return Ok(None),
        Some(path) => absolute_lexical(path)?,
        None => absolute_lexical(default_name)?,
    };
    Ok(Some(if path.is_dir() {
        path.join(default_name)
    } else {
        path
    }))
}

fn finish(
    table: &Table,
    output: Option<&Path>,
    options: &Cli,
    csv: bool,
    source: &str,
    failed: usize,
) -> Result<()> {
    let to_stderr = output.is_none();
    let summary = format!("{} in {source}", summary(table.rows.len(), table.found));
    let content = if csv { table.csv() } else { table.text() };
    if table.rows.is_empty() {
        note(to_stderr, format!("no links found in {source}"));
    } else if options.dry_run {
        let target = output.map_or_else(|| "standard output".to_owned(), display_path);
        note(to_stderr, format!("dry run — {summary} -> {target}"));
    } else if let Some(output) = output {
        confirm_replacement(output, options.yes)?;
        // A byte-order mark lets spreadsheet apps read the CSV as UTF-8.
        let bom = if csv { "\u{feff}" } else { "" };
        atomic_write(output, format!("{bom}{content}").as_bytes())?;
        note(
            false,
            format!("wrote {summary} -> {}", display_path(output)),
        );
    } else {
        io::stdout()
            .lock()
            .write_all(content.as_bytes())
            .context("could not write links")?;
        note(true, summary);
    }
    if failed > 0 {
        bail!("{failed} file(s) could not be read");
    }
    Ok(())
}

/// Progress goes to stderr when stdout carries the links themselves.
fn note(to_stderr: bool, message: impl Display) {
    if to_stderr {
        eprintln!("justlinks: {message}");
    } else {
        println!("justlinks: {message}");
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

/// Links in one file, chosen by content so misnamed files still work.
fn file_links(path: &Path) -> Result<Vec<Found>> {
    let mut head = Vec::new();
    File::open(path)
        .and_then(|file| file.take(1024).read_to_end(&mut head))
        .with_context(|| format!("{}: could not read", display_path(path)))?;
    let is_pdf = head.starts_with(b"%PDF-")
        || (extension(path).as_deref() == Some("pdf")
            && head.windows(5).any(|window| window == b"%PDF-"));
    if is_pdf {
        // PDF errors already name the file.
        return crate::pdf::link_occurrences(path);
    }
    let found = if head.starts_with(b"PK\x03\x04") {
        office::links(path)
    } else {
        text_file_links(path)
    };
    found.with_context(|| display_path(path))
}

fn text_file_links(path: &Path) -> Result<Vec<Found>> {
    if fs::metadata(path)?.len() > MAX_TEXT_BYTES {
        bail!("file is too large to scan");
    }
    let text = decode_text(&fs::read(path)?).ok_or(Unsupported)?;
    Ok(match extension(path).as_deref() {
        Some("csv") => delimited_links(&text, ','),
        Some("tsv") => delimited_links(&text, '\t'),
        _ => text_links(&text),
    })
}

/// A binary format JustLinks does not read; reported as skipped, not failed.
#[derive(Debug)]
struct Unsupported;

impl Display for Unsupported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "binary format not supported (legacy .xls/.doc/.ppt and password-protected Office files included)",
        )
    }
}

impl std::error::Error for Unsupported {}

/// Links in CSV or TSV rows. Fields are scanned separately, since URLs may
/// contain commas, and a URL without a label of its own takes its row's
/// first text field, such as an exercise name, as its title.
fn delimited_links(text: &str, delimiter: char) -> Vec<Found> {
    let mut found = Vec::new();
    for_each_row(text, delimiter, |line, fields| {
        let heading = fields
            .iter()
            .find(|field| text::urls(field).is_empty() && field.chars().any(char::is_alphabetic))
            .map(|field| title_from(field))
            .unwrap_or_default();
        for field in fields {
            for (_, url, label) in labeled_urls(field) {
                found.push(Found {
                    url: url.to_owned(),
                    title: if label.is_empty() {
                        heading.clone()
                    } else {
                        label
                    },
                    location: format!("line {line}"),
                });
            }
        }
    });
    found
}

/// Calls `handle` with each row's first line number and fields, honoring
/// quoted fields that contain delimiters, quotes, or line breaks.
fn for_each_row(text: &str, delimiter: char, mut handle: impl FnMut(usize, &[String])) {
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut line = 1;
    let mut start = 1;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted => {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' if field.is_empty() => quoted = true,
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                handle(start, &row);
                row.clear();
                line += 1;
                start = line;
            }
            '\r' if !quoted => {}
            _ if character == delimiter && !quoted => row.push(std::mem::take(&mut field)),
            _ => {
                if character == '\n' {
                    line += 1;
                }
                field.push(character);
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        handle(start, &row);
    }
}

/// Links in plain text, titled by their labels and located by line.
fn text_links(text: &str) -> Vec<Found> {
    let mut line = 1;
    let mut counted = 0;
    labeled_urls(text)
        .into_iter()
        .map(|(offset, url, title)| {
            line += text[counted..offset].matches('\n').count();
            counted = offset;
            Found {
                // HTML and XML escape `&` inside attribute values.
                url: url.replace("&amp;", "&"),
                title,
                location: format!("line {line}"),
            }
        })
        .collect()
}

/// Decodes UTF-8 or BOM-marked UTF-16 text; `None` for binary data.
fn decode_text(bytes: &[u8]) -> Option<String> {
    let utf16 = |bytes: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| from([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xff, 0xfe, rest @ ..] => Some(utf16(rest, u16::from_le_bytes)),
        [0xfe, 0xff, rest @ ..] => Some(utf16(rest, u16::from_be_bytes)),
        _ if bytes[..bytes.len().min(8192)].contains(&0) => None,
        _ => {
            let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(url: &str, title: &str, location: &str) -> Found {
        Found {
            url: url.into(),
            title: title.into(),
            location: location.into(),
        }
    }

    #[test]
    fn links_are_trimmed_and_deduplicated_in_first_seen_order() {
        let mut links = LinkSet::default();
        for link in [
            "https://b.test",
            "HTTPS://B.TEST",
            " https://a.test\0",
            "https://b.test",
            "",
            "\u{feff}",
        ] {
            links.add(link);
        }
        assert_eq!(links.text(), "https://b.test\nhttps://a.test\n");
        assert_eq!(links.summary(), "2 unique link(s) from 4 found");
    }

    #[test]
    fn csv_rows_keep_first_location_and_most_common_title() {
        let mut table = Table::default();
        table.add(&found("https://a.test", "hine C Press", "page 2"), "a.pdf");
        table.add(&found("https://b.test", "=cmd, \"x\"", "line 1"), "b.txt");
        for page in ["page 3", "page 4"] {
            table.add(
                &found("https://a.test", "Machine Chest Press", page),
                "a.pdf",
            );
        }
        table.add(&found("https://a.test", "", "page 5"), "a.pdf");
        assert_eq!(
            table.csv(),
            "url,title,file,location,occurrences\r\n\
             https://a.test,Machine Chest Press,a.pdf,page 2,4\r\n\
             https://b.test,\"'=cmd, \"\"x\"\"\",b.txt,line 1,1\r\n"
        );
        assert_eq!(table.text(), "https://a.test\nhttps://b.test\n");
    }

    #[test]
    fn every_collection_merges_youtube_variants_and_their_titles() {
        let mut table = Table::default();
        let mut links = LinkSet::default();
        for (url, title, file) in [
            ("https://youtu.be/AbC_dEf-123?t=30", "Clipped", "first.pdf"),
            (
                "https://youtube.com/shorts/AbC_dEf-123?si=token",
                "Full title",
                "second.xlsx",
            ),
            (
                "https://M.YOUTUBE.COM/watch?v=abc_def-123&list=PL1",
                "Full title",
                "third.txt",
            ),
        ] {
            table.add(&found(url, title, "page 1"), file);
            links.add(url);
        }
        assert_eq!(
            table.text(),
            "https://www.youtube.com/watch?v=AbC_dEf-123\n"
        );
        assert_eq!(links.text(), table.text());
        assert_eq!(links.summary(), "1 unique link(s) from 3 found");
        assert_eq!(
            table.csv(),
            "url,title,file,location,occurrences\r\nhttps://www.youtube.com/watch?v=AbC_dEf-123,Full title,first.pdf,page 1,3\r\n"
        );
    }

    #[test]
    fn text_links_report_their_lines() {
        let links = text_links(
            "intro\nSquat: https://a.test\n\n<a href=\"https://b.test/?x=1&amp;y=2\">Row</a>",
        );
        let summary: Vec<_> = links
            .iter()
            .map(|item| {
                (
                    item.url.as_str(),
                    item.title.as_str(),
                    item.location.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("https://a.test", "Squat", "line 2"),
                ("https://b.test/?x=1&y=2", "Row", "line 4"),
            ]
        );
    }

    #[test]
    fn csv_fields_are_scanned_separately_and_titled_by_their_row() {
        let csv = "Exercise,Video\r\nSquat,https://a.test/x,y\r\n\"Row, cable\",\"see: https://b.test\"\r\n\"multi\nline\",https://c.test\n";
        let summary: Vec<_> = delimited_links(csv, ',')
            .into_iter()
            .map(|item| (item.url, item.title, item.location))
            .collect();
        assert_eq!(
            summary,
            [
                ("https://a.test/x".into(), "Squat".into(), "line 2".into()),
                ("https://b.test".into(), "see".into(), "line 3".into()),
                (
                    "https://c.test".into(),
                    "multi line".into(),
                    "line 4".into()
                ),
            ]
        );
    }

    #[test]
    fn text_decoding_handles_boms_and_rejects_binary() {
        assert_eq!(decode_text(b"\xef\xbb\xbfhi").as_deref(), Some("hi"));
        assert_eq!(decode_text(b"\xff\xfeh\0i\0").as_deref(), Some("hi"));
        assert_eq!(decode_text(b"\xd0\xcf\x11\xe0\0\0"), None);
    }
}
