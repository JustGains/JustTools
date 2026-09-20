//! Link detection and titling in free text.

/// Link prefixes, matched case-insensitively.
const SCHEMES: [&str; 4] = ["https://", "http://", "ftp://", "mailto:"];
/// A bare web host, linked the way browsers and Office do.
const WEB_HOST: &str = "www.";
/// Longest title kept, in characters.
const MAX_TITLE: usize = 120;
/// Clause breaks; a label never reaches across one.
const BREAKS: [&str; 6] = [". ", "! ", "? ", "; ", " | ", " • "];
/// Separators trimmed from a label's end, as in `Squat: ` or `[Squat](`.
const TRAILING: &[char] = &[
    ':', '-', '–', '—', '|', '=', '>', '(', '[', ']', '•', '*', '#', ',', '"', '\'', '→',
];
/// Separators trimmed from a label's start, as in `- ` or `• `.
const LEADING: &[char] = &[
    '-', '–', '—', '*', '•', '·', '#', '>', '[', '|', ':', '"', '\'', ',',
];

/// Absolute URLs, `mailto:` addresses, and `www.` hosts in `text`, in order.
pub(crate) fn urls(text: &str) -> Vec<&str> {
    find(text).into_iter().map(|(_, link)| link).collect()
}

/// Each link in `text` with its byte offset and title: the text of an HTML
/// anchor, else the words before it on its line (`Squat: https://…`), else
/// the words after it.
pub(crate) fn labeled_urls(text: &str) -> Vec<(usize, &str, String)> {
    let found = find(text);
    found
        .iter()
        .enumerate()
        .map(|(index, (start, link))| {
            let end = start + link.len();
            let until = found.get(index + 1).map_or(text.len(), |(next, _)| *next);
            let title = anchor_text(text, *start, end)
                .or_else(|| non_empty(label_before(text, *start)))
                .unwrap_or_else(|| label_after(&text[end..until]));
            (*start, *link, title)
        })
        .collect()
}

/// A title from text shown for a link: the text itself, or when it contains
/// a URL, the label around that URL.
pub(crate) fn title_from(text: &str) -> String {
    match labeled_urls(text).into_iter().next() {
        Some((_, _, title)) => title,
        None => first_chars(&words(text), MAX_TITLE),
    }
}

/// The label before byte `start`: on the same line, after any earlier link
/// and the last clause break, without separators such as `:` or list bullets.
pub(crate) fn label_before(text: &str, start: usize) -> String {
    // A short window keeps long single-line files, such as minified HTML, linear.
    let mut from = start.saturating_sub(1024);
    while !text.is_char_boundary(from) {
        from += 1;
    }
    let mut region = &text[from..start];
    region = &region[region.rfind(['\n', '\r']).map_or(0, |index| index + 1)..];
    if let Some((offset, link)) = find(region).last() {
        region = &region[offset + link.len()..];
    }
    if let Some(cut) = BREAKS
        .iter()
        .filter_map(|separator| region.rfind(separator).map(|index| index + separator.len()))
        .max()
    {
        region = &region[cut..];
    }
    last_chars(&tidy(region), MAX_TITLE)
}

fn label_after(region: &str) -> String {
    let region = region.split(['\n', '\r']).next().unwrap_or_default();
    let cut = BREAKS
        .iter()
        .filter_map(|separator| region.find(separator))
        .min()
        .unwrap_or(region.len());
    first_chars(&tidy(&region[..cut]), MAX_TITLE)
}

/// The visible text of `<a href="URL">text</a>` when this link is the href.
fn anchor_text(text: &str, start: usize, end: usize) -> Option<String> {
    let before = text[..start].trim_end_matches(['"', '\'']).trim_end();
    let before = before.strip_suffix('=')?.trim_end();
    let attribute = before.get(before.len().checked_sub(4)?..)?;
    if !attribute.eq_ignore_ascii_case("href") {
        return None;
    }
    let after = &text[end..];
    let inner = &after[after.find('>')? + 1..];
    let mut limit = inner.len().min(4096);
    while !inner.is_char_boundary(limit) {
        limit -= 1;
    }
    let inner = &inner[..limit];
    let close = inner
        .char_indices()
        .find(|(index, _)| {
            inner[*index..]
                .get(..3)
                .is_some_and(|tag| tag.eq_ignore_ascii_case("</a"))
        })?
        .0;
    let mut visible = String::new();
    let mut in_tag = false;
    for character in inner[..close].chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => visible.push(character),
            _ => {}
        }
    }
    for (entity, character) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
    ] {
        visible = visible.replace(entity, character);
    }
    non_empty(first_chars(&words(&visible), MAX_TITLE))
}

fn tidy(text: &str) -> String {
    let text = words(text);
    let mut label = text
        .trim_end_matches(|character: char| {
            character.is_whitespace() || TRAILING.contains(&character)
        })
        .trim_start_matches(|character: char| {
            character.is_whitespace() || LEADING.contains(&character)
        });
    // List numbering such as `3. ` or `12) `.
    let unnumbered = label.trim_start_matches(|character: char| character.is_ascii_digit());
    if unnumbered.len() < label.len()
        && let Some(rest) = unnumbered
            .strip_prefix(". ")
            .or_else(|| unnumbered.strip_prefix(") "))
    {
        label = rest.trim_start();
    }
    label.to_owned()
}

fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// At most `limit` characters from the start, ending on a whole word.
fn first_chars(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        None => text.to_owned(),
        Some((cut, _)) => text[..cut]
            .rsplit_once(' ')
            .map_or(&text[..cut], |(head, _)| head)
            .to_owned(),
    }
}

/// At most `limit` characters from the end, starting on a whole word.
fn last_chars(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_owned();
    }
    let (cut, _) = text.char_indices().nth(count - limit).unwrap_or((0, ' '));
    text[cut..]
        .split_once(' ')
        .map_or(&text[cut..], |(_, tail)| tail)
        .to_owned()
}

/// Each link with its byte offset.
fn find(text: &str) -> Vec<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut links = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let Some(prefix) = prefix_at(bytes, index) else {
            index += 1;
            continue;
        };
        // Prefixes are ASCII, so `index` is on a character boundary.
        let candidate = &text[index..];
        let length = link_length(candidate);
        match candidate[..length].get(prefix.len()..) {
            Some(rest) if plausible(prefix, rest) => {
                links.push((index, &candidate[..length]));
                index += length;
            }
            _ => index += prefix.len(),
        }
    }
    links
}

fn prefix_at(bytes: &[u8], index: usize) -> Option<&'static str> {
    let previous = index.checked_sub(1).map(|index| bytes[index]);
    if previous.is_some_and(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    let starts_with = |prefix: &str| {
        bytes[index..]
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
    };
    if let Some(scheme) = SCHEMES.into_iter().find(|scheme| starts_with(scheme)) {
        return Some(scheme);
    }
    // A bare host must not continue an address, a path, or an email.
    if previous.is_some_and(|byte| b"@./-_".contains(&byte)) {
        return None;
    }
    if starts_with(WEB_HOST) {
        return Some(WEB_HOST);
    }
    [
        "youtu.be/",
        "youtube.com/",
        "m.youtube.com/",
        "music.youtube.com/",
        "gaming.youtube.com/",
        "youtube-nocookie.com/",
        "youtube.googleapis.com/",
    ]
    .into_iter()
    .find(|host| starts_with(host))
}

/// Byte length of the link that starts `candidate`: it ends at whitespace or
/// a delimiter, before an unbalanced closing bracket, and without trailing
/// sentence punctuation.
fn link_length(candidate: &str) -> usize {
    let mut open = [0usize; 3];
    let mut end = candidate.len();
    for (index, character) in candidate.char_indices() {
        if character.is_whitespace()
            || character.is_control()
            || matches!(character, '"' | '<' | '>' | '`' | '\\' | '|')
        {
            end = index;
            break;
        }
        if let Some(slot) = "([{".find(character) {
            open[slot] += 1;
        } else if let Some(slot) = ")]}".find(character) {
            if open[slot] == 0 {
                end = index;
                break;
            }
            open[slot] -= 1;
        }
    }
    candidate[..end]
        .trim_end_matches(['.', ',', ':', ';', '!', '?', '\'', '*', '~'])
        .len()
}

fn plausible(prefix: &str, rest: &str) -> bool {
    match prefix {
        "mailto:" => rest
            .split_once('@')
            .is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.')),
        WEB_HOST => {
            let host = rest.split(['/', '?', '#', ':']).next().unwrap_or_default();
            host.starts_with(|character: char| character.is_alphanumeric())
                && host
                    .rsplit_once('.')
                    .is_some_and(|(_, top)| !top.is_empty())
        }
        _ => rest.starts_with(|character: char| character.is_alphanumeric() || character == '['),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(text: &str) -> Vec<(&str, String)> {
        labeled_urls(text)
            .into_iter()
            .map(|(_, link, title)| (link, title))
            .collect()
    }

    #[test]
    fn finds_links_and_trims_sentence_punctuation() {
        assert_eq!(
            urls("See https://a.test/x, (https://b.test/wiki/Foo_(bar)). Or WWW.C.TEST!"),
            [
                "https://a.test/x",
                "https://b.test/wiki/Foo_(bar)",
                "WWW.C.TEST"
            ]
        );
    }

    #[test]
    fn bare_youtube_hosts_are_found_without_matching_other_addresses() {
        assert_eq!(
            urls(
                "youtu.be/AbC_dEf-123 YOUTUBE.COM/watch?v=AbC_dEf-123 m.youtube.com/shorts/AbC_dEf-123 user@youtu.be/AbC_dEf-123 fake.youtu.be/AbC_dEf-123 /youtu.be/AbC_dEf-123"
            ),
            [
                "youtu.be/AbC_dEf-123",
                "YOUTUBE.COM/watch?v=AbC_dEf-123",
                "m.youtube.com/shorts/AbC_dEf-123"
            ]
        );
    }

    #[test]
    fn stops_at_markup_and_markdown_delimiters() {
        assert_eq!(
            urls(
                r#"<a href="https://a.test/?q=1&amp;r=2">[x](https://b.test)</a> =HYPERLINK("https://c.test","go")"#
            ),
            [
                "https://a.test/?q=1&amp;r=2",
                "https://b.test",
                "https://c.test"
            ]
        );
    }

    #[test]
    fn rejects_fragments_and_embedded_prefixes() {
        assert!(
            urls("https:// nothing, www.localhost, mailto:, user@www.a.test, xhttp://a.test")
                .is_empty()
        );
        assert_eq!(
            urls("mailto:ada@example.test."),
            ["mailto:ada@example.test"]
        );
        assert_eq!(urls("é https://ünï.test/ö"), ["https://ünï.test/ö"]);
    }

    #[test]
    fn labels_come_from_the_words_before_a_link_on_its_line() {
        assert_eq!(
            titled("OMNI-GRIP LAT PULLDOWN: https://youtu.be/NDmJNX9JrLs?t=4m7s"),
            [(
                "https://youtu.be/NDmJNX9JrLs?t=4m7s",
                "OMNI-GRIP LAT PULLDOWN".to_owned()
            )]
        );
        assert_eq!(
            titled(
                "Warm up first. 1. Squat (high bar) - https://a.test\n- [Bench](https://b.test) | Row: https://c.test"
            ),
            [
                ("https://a.test", "Squat (high bar)".to_owned()),
                ("https://b.test", "Bench".to_owned()),
                ("https://c.test", "Row".to_owned()),
            ]
        );
    }

    #[test]
    fn labels_fall_back_to_following_words_and_anchor_text() {
        assert_eq!(
            titled("https://a.test - Deadlift demo. Next"),
            [("https://a.test", "Deadlift demo".to_owned())]
        );
        assert_eq!(
            titled(
                r#"<li><a class="x" href="https://a.test/v?id=1">Lat <b>Pulldown</b> &amp; Row</a></li>"#
            ),
            [("https://a.test/v?id=1", "Lat Pulldown & Row".to_owned())]
        );
        assert_eq!(
            title_from("  Machine   Chest Press "),
            "Machine Chest Press"
        );
        assert_eq!(title_from("https://youtu.be/x"), "");
    }
}
