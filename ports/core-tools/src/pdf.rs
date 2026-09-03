use crate::common::{
    absolute_lexical, atomic_write_with, collect_files, confirm, display_path, format_bytes,
    parse_cli, parse_piped_paths, read_stdin, same_path, stdin_is_terminal,
};
use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "justpdf",
    about = "Inspect, merge, split, extract, or rotate PDFs.",
    after_help = "PDF inputs are never removed. Page numbers are one-based; ranges look like 1-3,5,last.\nWith no command, one PDF shows info and multiple PDFs are merged to merged.pdf.\nThe installed JustTools multicall alias opens a saved-defaults launcher when run bare."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Merge/extract/rotate file, or split directory.
    #[arg(short = 'o', long, value_name = "PATH", global = true)]
    output: Option<PathBuf>,

    /// Pages for extract (required) or rotate (default: all).
    #[arg(short = 'p', long, value_name = "RANGE", global = true)]
    pages: Option<String>,

    /// Clockwise rotation: 90, 180, or 270 (default: 90).
    #[arg(short = 'd', long, default_value_t = 90, global = true)]
    degrees: u16,

    /// Include nested folders.
    #[arg(short = 'r', long, global = true)]
    recursive: bool,

    /// Replace existing outputs without asking.
    #[arg(short = 'y', long, global = true)]
    yes: bool,

    /// Show planned outputs without writing.
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,

    /// PDF files or folders.
    #[arg(value_name = "PDF")]
    inputs: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Merge PDFs in the given order.
    Merge {
        #[arg(value_name = "PDF", required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Write one PDF per page.
    Split {
        #[arg(value_name = "PDF", required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Copy selected pages to a new PDF.
    Extract {
        #[arg(value_name = "PDF", required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Rotate selected or all pages.
    Rotate {
        #[arg(value_name = "PDF", required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Show page count, sizes, and metadata.
    Info {
        #[arg(value_name = "PDF", required = true)]
        inputs: Vec<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Merge,
    Split,
    Extract,
    Rotate,
    Info,
}

pub fn run() -> Result<()> {
    let Some(options) = parse_cli::<Cli>()? else {
        return Ok(());
    };
    run_with(options)
}

fn run_with(options: Cli) -> Result<()> {
    if !matches!(options.degrees, 90 | 180 | 270) {
        bail!("degrees must be 90, 180, or 270");
    }
    let (explicit_operation, mut raw_inputs) = match options.command.as_ref() {
        Some(Command::Merge { inputs }) => (Some(Operation::Merge), inputs.clone()),
        Some(Command::Split { inputs }) => (Some(Operation::Split), inputs.clone()),
        Some(Command::Extract { inputs }) => (Some(Operation::Extract), inputs.clone()),
        Some(Command::Rotate { inputs }) => (Some(Operation::Rotate), inputs.clone()),
        Some(Command::Info { inputs }) => (Some(Operation::Info), inputs.clone()),
        None => (None, options.inputs.clone()),
    };
    if raw_inputs.is_empty() && !stdin_is_terminal() {
        raw_inputs = parse_piped_paths(&read_stdin()?);
    }
    if raw_inputs.is_empty() {
        bail!("provide at least one PDF");
    }
    let collected = collect_files(&raw_inputs, "pdf", options.recursive, None)?;
    for warning in &collected.warnings {
        eprintln!("justpdf: {warning}");
    }
    if collected.files.is_empty() {
        bail!("no PDF files found");
    }
    let files = collected.files;
    let operation = explicit_operation.unwrap_or(if files.len() == 1 {
        Operation::Info
    } else {
        Operation::Merge
    });
    if matches!(
        operation,
        Operation::Split | Operation::Extract | Operation::Rotate
    ) && files.len() != 1
    {
        bail!("{} needs exactly one PDF", operation.name());
    }

    match operation {
        Operation::Merge => merge(&files, &options),
        Operation::Split => split(&files[0], &options),
        Operation::Extract => extract(&files[0], &options),
        Operation::Rotate => rotate(&files[0], &options),
        Operation::Info => info(&files),
    }
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Split => "split",
            Self::Extract => "extract",
            Self::Rotate => "rotate",
            Self::Info => "info",
        }
    }
}

fn load_pdf(path: &Path) -> Result<Document> {
    let document =
        Document::load(path).map_err(|error| anyhow!("{}: {error}", display_path(path)))?;
    if document.is_encrypted() || document.was_encrypted() {
        bail!("{}: encrypted PDFs are not supported", display_path(path));
    }
    Ok(document)
}

fn confirm_outputs(outputs: &[PathBuf], yes: bool, dry_run: bool) -> Result<()> {
    let existing = outputs.iter().filter(|output| output.exists()).count();
    if existing > 0
        && !yes
        && !dry_run
        && !confirm(&format!("justpdf: replace {existing} existing output(s)"))?
    {
        bail!("cancelled");
    }
    Ok(())
}

fn save_pdf(document: &mut Document, output: &Path) -> Result<u64> {
    atomic_write_with(output, |file| {
        document
            .save_modern(file)
            .map_err(|error| anyhow!("could not encode PDF: {error}"))?;
        Ok(())
    })?;
    Ok(fs::metadata(output)?.len())
}

fn merge(files: &[PathBuf], options: &Cli) -> Result<()> {
    if files.len() < 2 {
        bail!("merge needs at least two PDFs");
    }
    let output = absolute_lexical(
        options
            .output
            .as_deref()
            .unwrap_or_else(|| Path::new("merged.pdf")),
    )?;
    if files.iter().any(|input| same_path(input, &output)) {
        bail!("merge output cannot also be an input");
    }
    if options.dry_run {
        println!(
            "justpdf: dry run — merge {} PDFs -> {}",
            files.len(),
            display_path(&output)
        );
        return Ok(());
    }
    confirm_outputs(std::slice::from_ref(&output), options.yes, false)?;
    let documents = files
        .iter()
        .map(|path| load_pdf(path))
        .collect::<Result<Vec<_>>>()?;
    let pages: usize = documents
        .iter()
        .map(|document| document.get_pages().len())
        .sum();
    let mut document = merge_documents(documents)?;
    let bytes = save_pdf(&mut document, &output)?;
    println!(
        "justpdf: merged {} PDFs / {pages} pages -> {} ({})",
        files.len(),
        display_path(&output),
        format_bytes(bytes)
    );
    Ok(())
}

fn split(input: &Path, options: &Cli) -> Result<()> {
    let source = load_pdf(input)?;
    let pages = source.get_pages().len();
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document");
    let default_directory = input.with_file_name(format!("{stem}-pages"));
    let output_directory =
        absolute_lexical(options.output.as_deref().unwrap_or(&default_directory))?;
    let width = std::cmp::max(3, pages.to_string().len());
    let outputs: Vec<_> = (1..=pages)
        .map(|page| output_directory.join(format!("{page:0width$}.pdf")))
        .collect();
    if options.dry_run {
        println!(
            "justpdf: dry run — split {pages} pages -> {}",
            display_path(&output_directory)
        );
        return Ok(());
    }
    confirm_outputs(&outputs, options.yes, false)?;
    let mut written = 0u64;
    for (index, output) in outputs.iter().enumerate() {
        let mut document = retain_pages(source.clone(), &[index as u32 + 1])?;
        written += save_pdf(&mut document, output)?;
    }
    println!(
        "justpdf: split {pages} page(s) -> {} ({})",
        display_path(&output_directory),
        format_bytes(written)
    );
    Ok(())
}

fn extract(input: &Path, options: &Cli) -> Result<()> {
    let expression = options
        .pages
        .as_deref()
        .ok_or_else(|| anyhow!("extract requires --pages RANGE"))?;
    let source = load_pdf(input)?;
    let selected = selected_pages(expression, source.get_pages().len())?;
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document");
    let safe_range: String = expression
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ',' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let default_output = input.with_file_name(format!("{stem}-pages-{safe_range}.pdf"));
    let output = absolute_lexical(options.output.as_deref().unwrap_or(&default_output))?;
    if same_path(input, &output) {
        bail!("extract output cannot overwrite its input");
    }
    if options.dry_run {
        println!(
            "justpdf: dry run — extract {} page(s) -> {}",
            selected.len(),
            display_path(&output)
        );
        return Ok(());
    }
    confirm_outputs(std::slice::from_ref(&output), options.yes, false)?;
    let page_documents = selected
        .iter()
        .map(|page| retain_pages(source.clone(), &[*page]))
        .collect::<Result<Vec<_>>>()?;
    let mut document = merge_documents(page_documents)?;
    let bytes = save_pdf(&mut document, &output)?;
    println!(
        "justpdf: extracted {} page(s) -> {} ({})",
        selected.len(),
        display_path(&output),
        format_bytes(bytes)
    );
    Ok(())
}

fn rotate(input: &Path, options: &Cli) -> Result<()> {
    let mut document = load_pdf(input)?;
    let selected = selected_pages(
        options.pages.as_deref().unwrap_or("all"),
        document.get_pages().len(),
    )?;
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document");
    let default_output = input.with_file_name(format!("{stem}-rotated.pdf"));
    let output = absolute_lexical(options.output.as_deref().unwrap_or(&default_output))?;
    if same_path(input, &output) {
        bail!("rotate output cannot overwrite its input");
    }
    if options.dry_run {
        println!(
            "justpdf: dry run — rotate {} page(s) {}° -> {}",
            selected.len(),
            options.degrees,
            display_path(&output)
        );
        return Ok(());
    }
    confirm_outputs(std::slice::from_ref(&output), options.yes, false)?;
    let pages = document.get_pages();
    for page_number in &selected {
        let page_id = pages[page_number];
        let previous = inherited_attribute(&document, page_id, b"Rotate")
            .and_then(|object| object.as_i64().ok())
            .unwrap_or(0);
        let angle = (previous + i64::from(options.degrees)).rem_euclid(360);
        document
            .get_object_mut(page_id)?
            .as_dict_mut()?
            .set("Rotate", angle);
    }
    document.compress();
    let bytes = save_pdf(&mut document, &output)?;
    println!(
        "justpdf: rotated {} page(s) {}° -> {} ({})",
        selected.len(),
        options.degrees,
        display_path(&output),
        format_bytes(bytes)
    );
    Ok(())
}

fn info(files: &[PathBuf]) -> Result<()> {
    for input in files {
        let document = load_pdf(input)?;
        let pages = document.get_pages();
        let mut sizes = Vec::new();
        for page_id in pages.values() {
            if let Some((width, height)) = page_size(&document, *page_id) {
                let size = format!("{:.0}×{:.0} pt", width.round(), height.round());
                if !sizes.contains(&size) {
                    sizes.push(size);
                }
            }
        }
        println!("{}:", display_path(input));
        println!("  pages: {}", pages.len());
        if !sizes.is_empty() {
            println!("  sizes: {}", sizes.join(", "));
        }
        println!("  bytes: {}", format_bytes(fs::metadata(input)?.len()));
        if let Some(info) = document
            .trailer
            .get(b"Info")
            .ok()
            .and_then(|object| resolve_object(&document, object).ok())
            .and_then(|object| object.as_dict().ok())
        {
            for (label, key) in [
                ("title", b"Title".as_slice()),
                ("author", b"Author"),
                ("subject", b"Subject"),
            ] {
                if let Some(value) = info.get(key).ok().and_then(pdf_string) {
                    println!("  {label}: {value}");
                }
            }
        }
    }
    Ok(())
}

fn selected_pages(expression: &str, page_count: usize) -> Result<Vec<u32>> {
    if expression.is_empty() || expression.eq_ignore_ascii_case("all") {
        return Ok((1..=page_count as u32).collect());
    }
    let parse_page = |token: &str| -> Result<u32> {
        if token.eq_ignore_ascii_case("last") {
            return Ok(page_count as u32);
        }
        let page = token
            .parse::<u32>()
            .map_err(|_| anyhow!("invalid page: {token}"))?;
        if page == 0 || page as usize > page_count {
            bail!("page must be from 1 to {page_count}: {token}");
        }
        Ok(page)
    };
    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    for raw_part in expression.split(',') {
        let part = raw_part.trim();
        if part.is_empty() {
            bail!("invalid page range: {expression}");
        }
        if let Some((left, right)) = part.split_once('-') {
            let first = parse_page(left.trim())?;
            let last = parse_page(right.trim())?;
            if first > last {
                bail!("page range must ascend: {part}");
            }
            for page in first..=last {
                if seen.insert(page) {
                    pages.push(page);
                }
            }
        } else {
            let page = parse_page(part)?;
            if seen.insert(page) {
                pages.push(page);
            }
        }
    }
    Ok(pages)
}

fn retain_pages(mut document: Document, retained: &[u32]) -> Result<Document> {
    let retained: HashSet<_> = retained.iter().copied().collect();
    let removed: Vec<_> = document
        .get_pages()
        .keys()
        .copied()
        .filter(|page| !retained.contains(page))
        .collect();
    document.delete_pages(&removed);
    if let Ok(catalog) = document.catalog_mut() {
        catalog.remove(b"Outlines");
        catalog.remove(b"PageLabels");
    }
    document.prune_objects();
    document.renumber_objects();
    document.compress();
    if document.get_pages().len() != retained.len() {
        bail!("PDF page tree could not be reduced safely");
    }
    Ok(document)
}

fn merge_documents(documents: Vec<Document>) -> Result<Document> {
    if documents.is_empty() {
        bail!("cannot create a PDF without pages");
    }
    let mut max_id = 1u32;
    let mut page_objects: Vec<(ObjectId, Object)> = Vec::new();
    let mut all_objects = BTreeMap::new();
    let mut output = Document::with_version("1.5");

    for mut document in documents {
        document.renumber_objects_with(max_id);
        max_id = document.max_id.saturating_add(1);
        for page_id in document.get_pages().into_values() {
            let mut page = document.get_object(page_id)?.as_dict()?.clone();
            for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
                if page.get(key).is_err()
                    && let Some(value) = inherited_attribute(&document, page_id, key)
                {
                    page.set(key, value);
                }
            }
            page_objects.push((page_id, Object::Dictionary(page)));
        }
        all_objects.extend(document.objects);
    }

    let mut catalog: Option<(ObjectId, Dictionary)> = None;
    let mut pages_id: Option<ObjectId> = None;
    for (object_id, object) in all_objects {
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                if catalog.is_none() {
                    catalog = Some((object_id, object.as_dict()?.clone()));
                }
            }
            b"Pages" => {
                pages_id.get_or_insert(object_id);
            }
            b"Page" | b"Outlines" | b"Outline" => {}
            _ => {
                output.objects.insert(object_id, object);
            }
        }
    }
    let pages_id = pages_id.ok_or_else(|| anyhow!("PDF pages root not found"))?;
    let (catalog_id, mut catalog) = catalog.ok_or_else(|| anyhow!("PDF catalog not found"))?;
    for (page_id, object) in &mut page_objects {
        object.as_dict_mut()?.set("Parent", pages_id);
        output.objects.insert(*page_id, object.clone());
    }
    let kids: Vec<_> = page_objects
        .iter()
        .map(|(page_id, _)| Object::Reference(*page_id))
        .collect();
    output.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => kids.len() as i64,
            "Kids" => kids,
        }),
    );
    catalog.set("Pages", pages_id);
    catalog.remove(b"Outlines");
    catalog.remove(b"PageLabels");
    output
        .objects
        .insert(catalog_id, Object::Dictionary(catalog));
    output.trailer.set("Root", catalog_id);
    output.max_id = output.objects.keys().map(|id| id.0).max().unwrap_or(0);
    output.renumber_objects();
    output.compress();
    Ok(output)
}

fn inherited_attribute(document: &Document, start: ObjectId, key: &[u8]) -> Option<Object> {
    let mut current = start;
    for _ in 0..64 {
        let dictionary = document.get_object(current).ok()?.as_dict().ok()?;
        if let Ok(value) = dictionary.get(key) {
            return Some(value.clone());
        }
        current = dictionary.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn resolve_object<'a>(document: &'a Document, object: &'a Object) -> lopdf::Result<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id),
        object => Ok(object),
    }
}

fn page_size(document: &Document, page_id: ObjectId) -> Option<(f64, f64)> {
    let media_box = inherited_attribute(document, page_id, b"MediaBox")?;
    let media_box = resolve_object(document, &media_box).ok()?.as_array().ok()?;
    if media_box.len() != 4 {
        return None;
    }
    let numbers: Vec<_> = media_box.iter().map(pdf_number).collect::<Option<_>>()?;
    Some((
        (numbers[2] - numbers[0]).abs(),
        (numbers[3] - numbers[1]).abs(),
    ))
}

fn pdf_number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    }
}

fn pdf_string(object: &Object) -> Option<String> {
    let bytes = object.as_str().ok()?;
    if bytes.starts_with(&[0xfe, 0xff]) && bytes.len() % 2 == 0 {
        let (pairs, _) = bytes[2..].as_chunks::<2>();
        let utf16: Vec<_> = pairs.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
        String::from_utf16(&utf16).ok()
    } else {
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges_are_one_based_ordered_and_deduplicated() {
        assert_eq!(selected_pages("1-3,2,last", 5).unwrap(), [1, 2, 3, 5]);
        assert_eq!(selected_pages("3,1", 3).unwrap(), [3, 1]);
        assert!(selected_pages("3-1", 3).is_err());
        assert!(selected_pages("0", 3).is_err());
    }

    #[test]
    fn merges_minimal_documents_in_input_order() {
        let first = fixture_document(100, 200);
        let second = fixture_document(300, 400);
        let merged = merge_documents(vec![first, second]).unwrap();
        let pages: Vec<_> = merged.get_pages().into_values().collect();
        assert_eq!(pages.len(), 2);
        assert_eq!(page_size(&merged, pages[0]), Some((100.0, 200.0)));
        assert_eq!(page_size(&merged, pages[1]), Some((300.0, 400.0)));
    }

    fn fixture_document(width: i64, height: i64) -> Document {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id =
            document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document
    }
}
