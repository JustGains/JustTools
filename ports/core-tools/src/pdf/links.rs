//! `justpdf links`: write each unique hyperlink once, in first-seen order.

use super::{
    Cli, confirm_outputs, dictionary, document_stem, load_pdf, pdf_number, pdf_string,
    resolve_object, selected_pages,
};
use crate::common::{absolute_lexical, atomic_write, display_path, same_path};
use crate::links::LinkSet;
use anyhow::{Result, bail};
use lopdf::{Document, Object, ObjectId};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Nesting limit for direct `/Next` action dictionaries.
const MAX_ACTION_DEPTH: usize = 32;

pub(super) fn run(input: &Path, options: &Cli) -> Result<()> {
    let document = load_pdf(input)?;
    let pages = document.get_pages();
    let selected = selected_pages(options.pages.as_deref().unwrap_or("all"), pages.len())?;
    let output = output_path(input, options)?;

    let mut links = LinkSet::default();
    for uri in uris(&document, &selected) {
        links.add(&uri);
    }
    if links.is_empty() {
        println!("justpdf: no links found");
        return Ok(());
    }
    if options.dry_run {
        println!(
            "justpdf: dry run — {} -> {}",
            links.summary(),
            display_path(&output)
        );
        return Ok(());
    }
    confirm_outputs(std::slice::from_ref(&output), options.yes, false)?;
    atomic_write(&output, links.text().as_bytes())?;
    println!(
        "justpdf: wrote {} -> {}",
        links.summary(),
        display_path(&output)
    );
    Ok(())
}

/// A URI action with where it was found: a link annotation's page and
/// rectangle, or a bookmark's title.
pub(super) struct Occurrence {
    pub(super) uri: String,
    pub(super) page: Option<u32>,
    pub(super) rect: Option<[f64; 4]>,
    pub(super) title: Option<String>,
}

/// URI actions on the selected pages, plus bookmark URIs when every page is
/// selected, in document order including repeats.
pub(super) fn occurrences(document: &Document, selected: &[u32]) -> Vec<Occurrence> {
    let pages = document.get_pages();
    let mut collector = Collector::default();
    for page in selected {
        if let Some(page_id) = pages.get(page) {
            collector.page(document, *page, *page_id);
        }
    }
    // Bookmarks belong to the whole document, not to any page range.
    if selected.len() == pages.len() {
        collector.outline(document);
    }
    collector.found
}

fn uris(document: &Document, selected: &[u32]) -> Vec<String> {
    occurrences(document, selected)
        .into_iter()
        .map(|occurrence| occurrence.uri)
        .collect()
}

fn output_path(input: &Path, options: &Cli) -> Result<PathBuf> {
    let name = format!("{}-links.txt", document_stem(input));
    let output = match options.output.as_deref() {
        Some(path) => {
            let path = absolute_lexical(path)?;
            if path.is_dir() { path.join(name) } else { path }
        }
        None => input.with_file_name(name),
    };
    if same_path(input, &output) {
        bail!("links output cannot overwrite its input");
    }
    Ok(output)
}

#[derive(Default)]
struct Collector {
    found: Vec<Occurrence>,
    /// Page, rectangle, and title recorded with the actions being visited.
    context: (Option<u32>, Option<[f64; 4]>, Option<String>),
}

impl Collector {
    fn page(&mut self, document: &Document, number: u32, page_id: ObjectId) {
        for annotation in document.get_page_annotations(page_id).unwrap_or_default() {
            if let Ok(action) = annotation.get(b"A") {
                let rect = annotation
                    .get(b"Rect")
                    .ok()
                    .and_then(|rect| resolve_object(document, rect).ok())
                    .and_then(|rect| rect.as_array().ok())
                    .and_then(|rect| {
                        let values: Vec<f64> = rect
                            .iter()
                            .filter_map(|value| resolve_object(document, value).ok())
                            .filter_map(pdf_number)
                            .collect();
                        <[f64; 4]>::try_from(values).ok()
                    });
                self.context = (Some(number), rect, None);
                self.action(document, action, &mut HashSet::new(), 0);
            }
        }
    }

    /// Visits bookmarks depth-first so children precede later siblings.
    fn outline(&mut self, document: &Document) {
        let first = document
            .catalog()
            .ok()
            .and_then(|catalog| catalog.get(b"Outlines").ok())
            .and_then(|outlines| dictionary(document, outlines))
            .and_then(|outlines| outlines.get(b"First").ok())
            .and_then(|first| first.as_reference().ok());
        let mut pending: Vec<ObjectId> = first.into_iter().collect();
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Ok(item) = document.get_dictionary(id) else {
                continue;
            };
            if let Ok(action) = item.get(b"A") {
                let title = item
                    .get(b"Title")
                    .ok()
                    .and_then(|title| resolve_object(document, title).ok())
                    .and_then(pdf_string);
                self.context = (None, None, title);
                self.action(document, action, &mut HashSet::new(), 0);
            }
            for key in [b"Next".as_slice(), b"First"] {
                if let Ok(next) = item.get(key).and_then(Object::as_reference) {
                    pending.push(next);
                }
            }
        }
    }

    /// Records a URI action and any actions chained after it; `visited`
    /// stops referenced `/Next` chains from cycling.
    fn action(
        &mut self,
        document: &Document,
        action: &Object,
        visited: &mut HashSet<ObjectId>,
        depth: usize,
    ) {
        if depth >= MAX_ACTION_DEPTH || action.as_reference().is_ok_and(|id| !visited.insert(id)) {
            return;
        }
        let Some(action) = dictionary(document, action) else {
            return;
        };
        if action
            .get(b"S")
            .and_then(Object::as_name)
            .is_ok_and(|kind| kind == b"URI")
            && let Some(uri) = action
                .get(b"URI")
                .ok()
                .and_then(|uri| resolve_object(document, uri).ok())
                .and_then(pdf_string)
        {
            let (page, rect, title) = self.context.clone();
            self.found.push(Occurrence {
                uri,
                page,
                rect,
                title,
            });
        }
        let Ok(next) = action.get(b"Next") else {
            return;
        };
        match resolve_object(document, next) {
            Ok(Object::Array(actions)) => {
                for next in actions {
                    self.action(document, next, visited, depth + 1);
                }
            }
            _ => self.action(document, next, visited, depth + 1),
        }
    }
}
