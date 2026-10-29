//! Text conversions for pasting the clipboard as a Markdown, HTML, or JSON file.
use serde_json::{Map, Value};

/// Rich text as Markdown; plain text is already Markdown.
pub fn markdown(html: Option<&str>, text: &str) -> Result<String, String> {
    let Some(html) = html else {
        return Ok(text.to_owned());
    };
    let converted = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "head", "meta", "link", "title"])
        .build()
        .convert(html)
        .map_err(|e| format!("cannot convert the copied HTML to Markdown: {e}"))?;
    let converted = converted.trim();
    Ok(if converted.is_empty() {
        text.to_owned()
    } else {
        format!("{converted}\n")
    })
}

/// A standalone UTF-8 page: the copied fragment, or plain text kept verbatim.
pub fn html(html: Option<&str>, text: &str) -> String {
    let body = match html {
        Some(html) => html.to_owned(),
        None => format!("<pre>{}</pre>", escape(text)),
    };
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>Pasted text</title>\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Copied data as pretty JSON: JSON itself, XML, CSV/TSV with a header row,
/// INI sections, or `key: value` lines; anything else becomes a JSON string.
pub fn json(text: &str) -> String {
    let trimmed = text.trim();
    let value = serde_json::from_str::<Value>(trimmed)
        .ok()
        .or_else(|| xml(trimmed))
        .or_else(|| table(trimmed))
        .or_else(|| ini(trimmed))
        .unwrap_or_else(|| Value::String(text.to_owned()));
    let mut output = serde_json::to_string_pretty(&value).unwrap_or_default();
    output.push('\n');
    output
}

fn xml(text: &str) -> Option<Value> {
    if !text.starts_with('<') {
        return None;
    }
    let document = roxmltree::Document::parse(text).ok()?;
    let root = document.root_element();
    let mut object = Map::new();
    object.insert(root.tag_name().name().to_owned(), element(root));
    Some(Value::Object(object))
}

fn element(node: roxmltree::Node) -> Value {
    let mut object = Map::new();
    for attribute in node.attributes() {
        object.insert(
            format!("@{}", attribute.name()),
            Value::String(attribute.value().to_owned()),
        );
    }
    let mut text = String::new();
    for child in node.children() {
        if child.is_element() {
            let name = child.tag_name().name().to_owned();
            let value = element(child);
            match object.get_mut(&name) {
                Some(Value::Array(items)) => items.push(value),
                Some(existing) => {
                    let first = existing.take();
                    *existing = Value::Array(vec![first, value]);
                }
                None => {
                    object.insert(name, value);
                }
            }
        } else if let Some(value) = child.text() {
            text.push_str(value);
        }
    }
    let text = text.trim();
    if object.is_empty() {
        return Value::String(text.to_owned());
    }
    if !text.is_empty() {
        object.insert("#text".into(), Value::String(text.to_owned()));
    }
    Value::Object(object)
}

/// One delimited row, honoring double-quoted fields and doubled quotes.
fn row(line: &str, delimiter: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                field.push('"');
                characters.next();
            }
            '"' if field.is_empty() || quoted => quoted = !quoted,
            _ if character == delimiter && !quoted => {
                fields.push(std::mem::take(&mut field).trim().to_owned())
            }
            _ => field.push(character),
        }
    }
    fields.push(field.trim().to_owned());
    fields
}

fn table(text: &str) -> Option<Value> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() < 2 {
        return None;
    }
    let delimiter = ['\t', ',', ';', '|'].into_iter().find(|delimiter| {
        let width = row(lines[0], *delimiter).len();
        width > 1
            && lines
                .iter()
                .all(|line| row(line, *delimiter).len() == width)
    })?;
    let header = row(lines[0], delimiter);
    if header.iter().any(String::is_empty) {
        return None;
    }
    let rows = lines[1..]
        .iter()
        .map(|line| {
            Value::Object(
                header
                    .iter()
                    .cloned()
                    .zip(row(line, delimiter).into_iter().map(scalar))
                    .collect(),
            )
        })
        .collect();
    Some(Value::Array(rows))
}

/// Numbers and booleans keep their type; everything else stays text.
fn scalar(value: String) -> Value {
    match value.as_str() {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => value
            .parse::<i64>()
            .map(Value::from)
            .ok()
            .or_else(|| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite() && !value.starts_with('.'))
                    .map(Value::from)
            })
            .unwrap_or(Value::String(value)),
    }
}

fn ini(text: &str) -> Option<Value> {
    let mut root = Map::new();
    let mut section: Option<String> = None;
    let mut pairs = 0;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim().to_owned();
            root.entry(name.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            section = Some(name);
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .or_else(|| line.split_once(": "))
            .or_else(|| line.strip_suffix(':').map(|key| (key, "")))?;
        let key = key.trim();
        if key.is_empty() || key.split_whitespace().count() > 4 {
            return None;
        }
        let value = scalar(value.trim().trim_matches('"').to_owned());
        let target = match &section {
            Some(name) => match root.get_mut(name) {
                Some(Value::Object(object)) => object,
                _ => return None,
            },
            None => &mut root,
        };
        target.insert(key.to_owned(), value);
        pairs += 1;
    }
    (pairs > 0).then_some(Value::Object(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_recognizes_json_tables_ini_xml_and_falls_back_to_a_string() {
        assert_eq!(json("{\"a\":1}"), "{\n  \"a\": 1\n}\n");
        let csv: Value = serde_json::from_str(&json(
            "name,age,city\n\"Lee, Ann\",41,\"Tampa\"\nBo,7,\"He said \"\"hi\"\"\"\n",
        ))
        .unwrap();
        assert_eq!(
            csv,
            serde_json::json!([
                {"name": "Lee, Ann", "age": 41, "city": "Tampa"},
                {"name": "Bo", "age": 7, "city": "He said \"hi\""}
            ])
        );
        let tsv: Value = serde_json::from_str(&json("a\tb\n1.5\ttrue")).unwrap();
        assert_eq!(tsv, serde_json::json!([{"a": 1.5, "b": true}]));
        let ini: Value = serde_json::from_str(&json(
            "# settings\ntitle = Demo\n[server]\nport=8080\nhost: local",
        ))
        .unwrap();
        assert_eq!(
            ini,
            serde_json::json!({"title": "Demo", "server": {"port": 8080, "host": "local"}})
        );
        let xml: Value = serde_json::from_str(&json(
            "<list kind=\"x\"><item>one</item><item>two</item><note>n</note></list>",
        ))
        .unwrap();
        assert_eq!(
            xml,
            serde_json::json!({"list": {"@kind": "x", "item": ["one", "two"], "note": "n"}})
        );
        assert_eq!(json("Just a sentence."), "\"Just a sentence.\"\n");
    }

    #[test]
    fn markdown_and_html_keep_the_copied_structure() {
        let markdown = markdown(
            Some("<h2>Title</h2><p>Some <b>bold</b> <a href=\"https://a.test\">link</a></p><script>x()</script>"),
            "Title Some bold link",
        )
        .unwrap();
        assert!(markdown.contains("## Title"), "{markdown}");
        assert!(markdown.contains("**bold**"), "{markdown}");
        assert!(markdown.contains("[link](https://a.test)"), "{markdown}");
        assert!(!markdown.contains("x()"), "{markdown}");
        assert_eq!(
            super::markdown(None, "plain *text*").unwrap(),
            "plain *text*"
        );
        assert!(html(None, "a < b & \"c\"").contains("<pre>a &lt; b &amp; &quot;c&quot;</pre>"));
        assert!(html(Some("<b>x</b>"), "x").contains("<body>\n<b>x</b>\n</body>"));
    }
}
