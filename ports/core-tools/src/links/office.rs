//! Links in Office Open XML packages (.xlsx, .docx, .pptx), with titles.
//!
//! Worksheets are read in tab order and cell by cell; other parts in natural
//! name order (`slide2` before `slide10`) and paragraph by paragraph. Links
//! come from hyperlink relationships, HYPERLINK formulas and fields, and URLs
//! typed in text.

use super::{Found, text, title_from};
use anyhow::{Context, Result, bail};
use roxmltree::{Document, Node};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

/// Relationship namespaces for Transitional and Strict Office Open XML.
const RELATIONSHIPS: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
];
const WORKBOOK: &str = "xl/workbook.xml";
const SHARED_STRINGS: &str = "xl/sharedStrings.xml";
/// Largest package part read; guards against ZIP bombs.
const MAX_PART_BYTES: u64 = 256 << 20;
/// Elements whose text runs form one string: spreadsheet strings, document
/// and slide paragraphs, and comments.
const CONTAINERS: [&str; 4] = ["si", "is", "p", "text"];

struct Relationship {
    id: String,
    target: String,
    hyperlink: bool,
    used: bool,
}

pub(super) fn links(path: &Path) -> Result<Vec<Found>> {
    let mut archive =
        ZipArchive::new(File::open(path)?).context("not a readable Office document")?;
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    if !names.iter().any(|name| name == "[Content_Types].xml") {
        bail!("this ZIP archive is not an Office Open XML document");
    }
    let has = |name: &str| names.iter().any(|candidate| candidate == name);
    let shared = if has(SHARED_STRINGS) {
        shared_strings(&read(&mut archive, SHARED_STRINGS)?).context(SHARED_STRINGS)?
    } else {
        Vec::new()
    };
    let sheets = if has(WORKBOOK) {
        sheet_names(&mut archive, &names)?
    } else {
        Vec::new()
    };

    let mut parts: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| {
            name.ends_with(".xml") && !matches!(*name, "[Content_Types].xml" | SHARED_STRINGS)
        })
        .collect();
    // Worksheets first in tab order, then every other part.
    parts.sort_by_cached_key(|part| {
        let tab = sheets
            .iter()
            .position(|(sheet, _)| sheet == part)
            .unwrap_or(usize::MAX);
        (tab, natural_key(part))
    });

    let mut found = Vec::new();
    let mut owned = HashSet::new();
    for part in parts {
        let rels = relationships_part(part);
        let mut relationships = if has(&rels) {
            let xml = read(&mut archive, &rels)?;
            owned.insert(rels.clone());
            relationships(&xml).with_context(|| rels.clone())?
        } else {
            Vec::new()
        };
        relationships.retain(|relationship| relationship.hyperlink);
        let xml = read(&mut archive, part)?;
        let result = match sheets.iter().find(|(sheet, _)| sheet == part) {
            Some((_, name)) => worksheet(&xml, &shared, &mut relationships, name, &mut found),
            None => document_part(&xml, &mut relationships, part, &mut found),
        };
        result.with_context(|| part.to_owned())?;
        // A hyperlink relationship nothing references is still in the file.
        found.extend(
            relationships
                .into_iter()
                .filter(|relationship| !relationship.used)
                .map(|relationship| Found {
                    url: relationship.target,
                    title: String::new(),
                    location: part_location(part, None),
                }),
        );
    }
    for rels in names
        .iter()
        .filter(|name| name.ends_with(".rels") && !owned.contains(*name))
    {
        let xml = read(&mut archive, rels)?;
        for relationship in relationships(&xml).with_context(|| rels.clone())? {
            if relationship.hyperlink {
                found.push(Found {
                    url: relationship.target,
                    title: String::new(),
                    location: rels.clone(),
                });
            }
        }
    }
    Ok(found)
}

fn read(archive: &mut ZipArchive<File>, name: &str) -> Result<String> {
    let entry = archive
        .by_name(name)
        .with_context(|| format!("{name} could not be opened"))?;
    let mut bytes = Vec::new();
    entry
        .take(MAX_PART_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("{name} could not be read"))?;
    if bytes.len() as u64 > MAX_PART_BYTES {
        bail!("{name} is too large");
    }
    let text = String::from_utf8(bytes).with_context(|| format!("{name} is not UTF-8"))?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(text) => text.to_owned(),
        None => text,
    })
}

/// Part names that sort numbers by value: `sheet2.xml` before `sheet10.xml`.
fn natural_key(name: &str) -> String {
    let mut key = String::with_capacity(name.len());
    let mut digits = String::new();
    for character in name.chars().chain(std::iter::once('\0')) {
        if character.is_ascii_digit() {
            digits.push(character);
            continue;
        }
        if !digits.is_empty() {
            key.push_str(&format!("{digits:0>20}"));
            digits.clear();
        }
        key.push(character);
    }
    key
}

fn relationships_part(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((folder, file)) => format!("{folder}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

fn relationships(xml: &str) -> Result<Vec<Relationship>> {
    let document = Document::parse(xml)?;
    Ok(document
        .descendants()
        .filter(|node| node.tag_name().name() == "Relationship")
        .filter_map(|node| {
            Some(Relationship {
                id: node.attribute("Id")?.to_owned(),
                target: node.attribute("Target")?.to_owned(),
                hyperlink: node.attribute("TargetMode") == Some("External")
                    && node
                        .attribute("Type")
                        .is_some_and(|kind| kind.ends_with("/hyperlink")),
                used: false,
            })
        })
        .collect())
}

fn relationship_id<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| {
            attribute.name() == "id"
                && attribute
                    .namespace()
                    .is_some_and(|namespace| RELATIONSHIPS.contains(&namespace))
        })
        .map(|attribute| attribute.value())
}

/// Takes the hyperlink with `id`, marking it referenced.
fn take_hyperlink(relationships: &mut [Relationship], id: &str) -> Option<String> {
    let relationship = relationships
        .iter_mut()
        .find(|relationship| relationship.id == id)?;
    relationship.used = true;
    Some(relationship.target.clone())
}

/// Worksheet part names and sheet names, in tab order.
fn sheet_names(archive: &mut ZipArchive<File>, names: &[String]) -> Result<Vec<(String, String)>> {
    let rels = relationships_part(WORKBOOK);
    if !names.contains(&rels) {
        return Ok(Vec::new());
    }
    let targets: HashMap<String, String> = relationships(&read(archive, &rels)?)
        .with_context(|| rels.clone())?
        .into_iter()
        .map(|relationship| (relationship.id, relationship.target))
        .collect();
    let workbook = read(archive, WORKBOOK)?;
    let document = Document::parse(&workbook).context(WORKBOOK)?;
    Ok(document
        .descendants()
        .filter(|node| node.tag_name().name() == "sheet")
        .filter_map(|sheet| {
            let target = targets.get(relationship_id(sheet)?)?;
            // Targets are relative to xl/ unless absolute within the package.
            let part = match target.strip_prefix('/') {
                Some(absolute) => absolute.to_owned(),
                None => format!("xl/{target}"),
            };
            Some((part, sheet.attribute("name")?.to_owned()))
        })
        .collect())
}

fn shared_strings(xml: &str) -> Result<Vec<String>> {
    let document = Document::parse(xml)?;
    Ok(document
        .root_element()
        .children()
        .filter(|node| node.tag_name().name() == "si")
        .map(string_text)
        .collect())
}

/// The text of a spreadsheet string, without phonetic guides.
fn string_text(node: Node) -> String {
    node.descendants()
        .filter(|node| {
            node.tag_name().name() == "t"
                && !node
                    .ancestors()
                    .any(|ancestor| ancestor.tag_name().name() == "rPh")
        })
        .filter_map(|node| node.text())
        .collect()
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.tag_name().name() == name)
}

fn worksheet(
    xml: &str,
    shared: &[String],
    relationships: &mut [Relationship],
    sheet: &str,
    found: &mut Vec<Found>,
) -> Result<()> {
    let document = Document::parse(xml)?;
    // Hyperlinks follow the cells in the file; attach them to their cells.
    let mut attached: Vec<(String, String, String)> = Vec::new();
    for hyperlink in document
        .descendants()
        .filter(|node| node.tag_name().name() == "hyperlink")
    {
        if let Some(url) =
            relationship_id(hyperlink).and_then(|id| take_hyperlink(relationships, id))
        {
            let reference = hyperlink.attribute("ref").unwrap_or_default();
            // A range such as A1:B2 links its first cell.
            let first = reference.split(':').next().unwrap_or_default();
            let display = hyperlink.attribute("display").unwrap_or_default();
            attached.push((first.to_ascii_uppercase(), url, display.to_owned()));
        }
    }
    let quoted = if sheet
        .chars()
        .all(|character| character.is_alphanumeric() || character == '_')
    {
        sheet.to_owned()
    } else {
        format!("'{}'", sheet.replace('\'', "''"))
    };
    let mut placed = HashSet::new();
    for row in document
        .descendants()
        .filter(|node| node.tag_name().name() == "row")
    {
        // The row's first text cell, such as an exercise name, titles bare URLs.
        let mut heading = String::new();
        for cell in row.children().filter(|node| node.tag_name().name() == "c") {
            let reference = cell.attribute("r").unwrap_or_default().to_ascii_uppercase();
            let location = format!("{quoted}!{reference}");
            let text = cell_text(cell, shared);
            let own = title_from(&text);
            let mut push = |url: String, titles: [&str; 3]| {
                let title = titles
                    .into_iter()
                    .find(|title| !title.is_empty())
                    .unwrap_or_default()
                    .to_owned();
                found.push(Found {
                    url,
                    title,
                    location: location.clone(),
                });
            };
            for (_, url, display) in attached.iter().filter(|(cell, ..)| *cell == reference) {
                push(
                    url.clone(),
                    [&own, &title_from(display), &heading].map(String::as_str),
                );
            }
            placed.insert(reference.clone());
            if let Some(formula) = child(cell, "f").and_then(|formula| formula.text()) {
                // HYPERLINK's second argument is its friendly name.
                let friendly = string_literals(formula)
                    .get(1)
                    .map(|name| title_from(name))
                    .unwrap_or_default();
                for url in text::urls(formula) {
                    push(
                        url.to_owned(),
                        [&friendly, &own, &heading].map(String::as_str),
                    );
                }
            }
            for (_, url, label) in text::labeled_urls(&text) {
                push(url.to_owned(), [label.as_str(), heading.as_str(), ""]);
            }
            if heading.is_empty()
                && text::urls(&text).is_empty()
                && text.chars().any(char::is_alphabetic)
            {
                heading = own;
            }
        }
    }
    for (reference, url, display) in attached {
        if !placed.contains(&reference) {
            found.push(Found {
                url,
                title: title_from(&display),
                location: format!("{quoted}!{reference}"),
            });
        }
    }
    Ok(())
}

/// A cell's displayed text: shared, inline, or cached formula strings, or its value.
fn cell_text(cell: Node, shared: &[String]) -> String {
    let value = child(cell, "v")
        .and_then(|value| value.text())
        .unwrap_or_default();
    match cell.attribute("t") {
        Some("s") => value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| shared.get(index))
            .cloned()
            .unwrap_or_default(),
        Some("inlineStr") => child(cell, "is").map(string_text).unwrap_or_default(),
        _ => value.to_owned(),
    }
}

/// Double-quoted string literals in a formula, with `""` unescaped.
fn string_literals(formula: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut characters = formula.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '"' {
            continue;
        }
        let mut literal = String::new();
        while let Some(character) = characters.next() {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    literal.push('"');
                } else {
                    break;
                }
            } else {
                literal.push(character);
            }
        }
        literals.push(literal);
    }
    literals
}

/// A readable location for a document part, with an optional paragraph.
fn part_location(part: &str, paragraph: Option<usize>) -> String {
    let stem = part
        .rsplit('/')
        .next()
        .unwrap_or(part)
        .trim_end_matches(".xml");
    let number = stem.trim_start_matches(|character: char| !character.is_ascii_digit());
    let base = if part.starts_with("ppt/slides/") {
        format!("slide {number}")
    } else if part.starts_with("ppt/notesSlides/") {
        format!("notes {number}")
    } else if part == "word/document.xml" {
        String::new()
    } else {
        stem.to_owned()
    };
    match (paragraph, part.starts_with("word/")) {
        (Some(paragraph), true) if base.is_empty() => format!("paragraph {paragraph}"),
        (Some(paragraph), true) => format!("{base} paragraph {paragraph}"),
        _ if base.is_empty() => "document".into(),
        _ => base,
    }
}

/// A link inside a paragraph, placed at the text offset where it begins.
struct Span {
    offset: usize,
    url: String,
    covered: String,
}

fn document_part(
    xml: &str,
    relationships: &mut [Relationship],
    part: &str,
    found: &mut Vec<Found>,
) -> Result<()> {
    let document = Document::parse(xml)?;
    let mut paragraph = 0;
    for node in document.descendants().filter(Node::is_element) {
        let container = nearest_container(node);
        if CONTAINERS.contains(&node.tag_name().name()) {
            paragraph += 1;
            let location = part_location(part, Some(paragraph));
            let (text, spans) = paragraph_text(node, relationships);
            let mut items: Vec<(usize, Found)> = spans
                .into_iter()
                .map(|span| {
                    let title = Some(title_from(&span.covered))
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| text::label_before(&text, span.offset));
                    (
                        span.offset,
                        Found {
                            url: span.url,
                            title,
                            location: location.clone(),
                        },
                    )
                })
                .collect();
            items.extend(
                text::labeled_urls(&text)
                    .into_iter()
                    .map(|(offset, url, title)| {
                        (
                            offset,
                            Found {
                                url: url.to_owned(),
                                title,
                                location: location.clone(),
                            },
                        )
                    }),
            );
            items.sort_by_key(|(offset, _)| *offset);
            found.extend(items.into_iter().map(|(_, item)| item));
        } else if container.is_none() {
            // Links on shapes or pictures, outside any paragraph.
            if let Some(url) =
                relationship_id(node).and_then(|id| take_hyperlink(relationships, id))
            {
                found.push(Found {
                    url,
                    title: String::new(),
                    location: part_location(part, None),
                });
            }
            if node.tag_name().name() == "t" {
                for (_, url, title) in text::labeled_urls(node.text().unwrap_or_default()) {
                    found.push(Found {
                        url: url.to_owned(),
                        title,
                        location: part_location(part, None),
                    });
                }
            }
        }
    }
    Ok(())
}

/// A paragraph's text, with its hyperlinks and HYPERLINK fields placed by offset.
fn paragraph_text(paragraph: Node, relationships: &mut [Relationship]) -> (String, Vec<Span>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    // Open Word fields: their instruction and where their displayed text starts.
    let mut fields: Vec<(String, Option<usize>)> = Vec::new();
    for node in paragraph.descendants().filter(Node::is_element) {
        if nearest_container(node) != Some(paragraph) {
            continue;
        }
        if let Some(url) = relationship_id(node).and_then(|id| take_hyperlink(relationships, id)) {
            spans.push(Span {
                offset: text.len(),
                url,
                covered: covered_text(node, paragraph),
            });
        }
        match node.tag_name().name() {
            "t" => text.push_str(node.text().unwrap_or_default()),
            "tab" => text.push('\t'),
            "br" | "cr" => text.push('\n'),
            "fldSimple" => {
                for url in text::urls(node_attribute_value(node, "instr").unwrap_or_default()) {
                    spans.push(Span {
                        offset: text.len(),
                        url: url.to_owned(),
                        covered: covered_text(node, paragraph),
                    });
                }
            }
            "instrText" => {
                if let Some((instruction, _)) = fields.last_mut() {
                    instruction.push_str(node.text().unwrap_or_default());
                }
            }
            "fldChar" => match node_attribute_value(node, "fldCharType") {
                Some("begin") => fields.push((String::new(), None)),
                Some("separate") => {
                    if let Some((_, start)) = fields.last_mut() {
                        *start = Some(text.len());
                    }
                }
                Some("end") => {
                    if let Some((instruction, start)) = fields.pop()
                        && instruction.trim_start().starts_with("HYPERLINK")
                    {
                        let start = start.unwrap_or(text.len());
                        for url in text::urls(&instruction) {
                            spans.push(Span {
                                offset: start,
                                url: url.to_owned(),
                                covered: text[start..].to_owned(),
                            });
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    (text, spans)
}

/// An attribute by local name, whatever its namespace prefix.
fn node_attribute_value<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| attribute.name() == name)
        .map(|attribute| attribute.value())
}

/// The text a link element displays: its own runs, or for a run property
/// such as PowerPoint's `hlinkClick`, the enclosing run's text.
fn covered_text(node: Node, paragraph: Node) -> String {
    let runs = |element: Node| -> String {
        element
            .descendants()
            .filter(|node| node.tag_name().name() == "t")
            .filter_map(|node| node.text())
            .collect()
    };
    node.ancestors()
        .take_while(|ancestor| *ancestor != paragraph)
        .map(runs)
        .find(|text| !text.is_empty())
        .unwrap_or_default()
}

/// The nearest enclosing text container, excluding `node` itself.
fn nearest_container<'a, 'input>(node: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    node.ancestors()
        .skip(1)
        .find(|ancestor| CONTAINERS.contains(&ancestor.tag_name().name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

    fn hyperlink(id: &str, target: &str) -> Relationship {
        Relationship {
            id: id.into(),
            target: target.into(),
            hyperlink: true,
            used: false,
        }
    }

    fn summary(found: &[Found]) -> Vec<(&str, &str, &str)> {
        found
            .iter()
            .map(|item| {
                (
                    item.url.as_str(),
                    item.title.as_str(),
                    item.location.as_str(),
                )
            })
            .collect()
    }

    #[test]
    fn part_names_sort_numbers_by_value() {
        let mut parts = ["xl/worksheets/sheet10.xml", "xl/worksheets/sheet2.xml"];
        parts.sort_by_cached_key(|name| natural_key(name));
        assert_eq!(
            parts,
            ["xl/worksheets/sheet2.xml", "xl/worksheets/sheet10.xml"]
        );
        assert_eq!(
            relationships_part("word/document.xml"),
            "word/_rels/document.xml.rels"
        );
        assert_eq!(part_location("ppt/slides/slide12.xml", Some(3)), "slide 12");
        assert_eq!(part_location("word/document.xml", Some(3)), "paragraph 3");
    }

    #[test]
    fn word_paragraphs_title_hyperlinks_fields_and_typed_urls() {
        let mut relationships = vec![hyperlink("rId1", "https://rel.test/a?b=1&c=2")];
        let xml = format!(
            r#"<w:document xmlns:w="w" xmlns:r="{R}"><w:body>
            <w:p><w:r><w:t>Read https://split</w:t></w:r><w:r><w:t>.test/doc now</w:t></w:r></w:p>
            <w:p><w:r><w:t>Squat: </w:t></w:r><w:hyperlink r:id="rId1"><w:r><w:t>video</w:t></w:r></w:hyperlink></w:p>
            <w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> HYPERLINK "https://field.test" </w:instrText></w:r>
                <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Field title</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
            <w:p><w:r><w:t>Row</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>https://tab.test</w:t></w:r></w:p>
            </w:body></w:document>"#
        );
        let mut found = Vec::new();
        document_part(&xml, &mut relationships, "word/document.xml", &mut found).unwrap();
        assert_eq!(
            summary(&found),
            [
                ("https://split.test/doc", "Read", "paragraph 1"),
                ("https://rel.test/a?b=1&c=2", "video", "paragraph 2"),
                ("https://field.test", "Field title", "paragraph 3"),
                ("https://tab.test", "Row", "paragraph 4"),
            ]
        );
        assert!(relationships[0].used);
    }

    #[test]
    fn slide_runs_title_their_click_links() {
        let mut relationships = vec![hyperlink("rId2", "https://slide.test")];
        let xml = format!(
            r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="{R}"><a:p><a:r><a:rPr><a:hlinkClick r:id="rId2"/></a:rPr><a:t>Watch the demo</a:t></a:r></a:p></p:sld>"#
        );
        let mut found = Vec::new();
        document_part(
            &xml,
            &mut relationships,
            "ppt/slides/slide3.xml",
            &mut found,
        )
        .unwrap();
        assert_eq!(
            summary(&found),
            [("https://slide.test", "Watch the demo", "slide 3")]
        );
    }

    #[test]
    fn worksheet_cells_use_display_text_friendly_names_and_row_headings() {
        let shared = vec![
            "Lat Pulldown".to_owned(),
            "OMNI-GRIP LAT PULLDOWN: https://youtu.be/NDmJNX9JrLs?t=4m7s".to_owned(),
        ];
        let mut relationships = vec![hyperlink("rId1", "https://rel.test/row")];
        let xml = format!(
            r#"<worksheet xmlns:r="{R}"><sheetData>
            <row r="2"><c r="A2" t="s"><v>0</v></c><c r="B2" t="inlineStr"><is><t>3 x 10</t></is></c>
                <c r="C2" t="str"><f>HYPERLINK("https://formula.test","Watch ""demo""")</f><v>Watch "demo"</v></c>
                <c r="D2" t="inlineStr"><is><t>https://typed.test</t></is></c><c r="E2"/></row>
            <row r="3"><c r="A3" t="s"><v>1</v></c></row>
            </sheetData><hyperlinks><hyperlink ref="E2" r:id="rId1"/><hyperlink ref="A9" location="Other!A1"/></hyperlinks></worksheet>"#
        );
        let mut found = Vec::new();
        worksheet(&xml, &shared, &mut relationships, "Week 1", &mut found).unwrap();
        assert_eq!(
            summary(&found),
            [
                ("https://formula.test", "Watch \"demo\"", "'Week 1'!C2"),
                ("https://typed.test", "Lat Pulldown", "'Week 1'!D2"),
                ("https://rel.test/row", "Lat Pulldown", "'Week 1'!E2"),
                (
                    "https://youtu.be/NDmJNX9JrLs?t=4m7s",
                    "OMNI-GRIP LAT PULLDOWN",
                    "'Week 1'!A3"
                ),
            ]
        );
    }
}
