//! Shared identity and output rules for every link collection.

/// Match the entire URL without casing differences, but retain the first
/// spelling. YouTube videos additionally share one parameter-free watch URL.
pub(super) fn link(value: &str) -> (String, String) {
    let url = video_id(value, 0).map_or_else(
        || value.to_owned(),
        |id| format!("https://www.youtube.com/watch?v={id}"),
    );
    (url.to_lowercase(), url)
}

fn video_id(value: &str, depth: usize) -> Option<String> {
    if depth >= 5 {
        return None;
    }
    let value = value.trim();
    let address = ["https://", "http://", "//"]
        .into_iter()
        .find_map(|prefix| {
            value
                .get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .map(|_| &value[prefix.len()..])
        })
        .unwrap_or(value);
    let boundary = address.find(['/', '?', '#']).unwrap_or(address.len());
    let host = address[..boundary].to_ascii_lowercase();
    if host.contains(['@', '\\']) {
        return None;
    }
    let host = host
        .strip_suffix(":443")
        .or_else(|| host.strip_suffix(":80"))
        .unwrap_or(&host);
    let host = host.trim_end_matches('.');
    let short = matches!(host, "youtu.be" | "www.youtu.be");
    if !short
        && ![
            "youtube.com",
            "youtube-nocookie.com",
            "youtube.googleapis.com",
        ]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    {
        return None;
    }
    let tail = &address[boundary..];
    let (resource, fragment) = tail.split_once('#').unwrap_or((tail, ""));
    let (path, query) = resource.split_once('?').unwrap_or((resource, ""));
    let path = decode(path)?;
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    let route = parts[0].to_ascii_lowercase();
    if short {
        return (parts.len() == 1).then(|| valid_id(parts[0])).flatten();
    }
    if route == "watch" && parts.len() == 1 {
        return parameter(query, "v").and_then(|id| valid_id(&id));
    }
    if matches!(route.as_str(), "embed" | "v" | "e" | "shorts" | "live") && parts.len() == 2 {
        if route == "embed" && parts[1].eq_ignore_ascii_case("videoseries") {
            return None;
        }
        return valid_id(parts[1]);
    }
    if route == "watch" && parts.len() == 3 && parts[1].eq_ignore_ascii_case("v") {
        return valid_id(parts[2]);
    }
    // YouTube's share/redirect wrappers carry a percent-encoded destination.
    let keys: &[&str] = match route.as_str() {
        "attribution_link" => &["u"],
        "redirect" => &["q", "url"],
        "oembed" => &["url"],
        _ => &[],
    };
    for key in keys {
        if let Some(target) = parameter(query, key) {
            let target = if target.starts_with('/') && !target.starts_with("//") {
                format!("https://www.youtube.com{target}")
            } else {
                target
            };
            if let Some(id) = video_id(&target, depth + 1) {
                return Some(id);
            }
        }
    }
    // Old hash-based watch links, without interpreting timestamps as IDs.
    if path.trim_matches('/').is_empty() {
        let fragment = fragment.strip_prefix('!').unwrap_or(fragment);
        if fragment.starts_with('/') {
            return video_id(&format!("https://www.youtube.com{fragment}"), depth + 1);
        }
    }
    None
}

fn valid_id(value: &str) -> Option<String> {
    (value.len() == 11
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)))
    .then(|| value.to_owned())
}

fn parameter(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        decode(key)?
            .eq_ignore_ascii_case(name)
            .then(|| decode(value))?
    })
}

/// Decode a URL component once, rejecting malformed escapes and UTF-8.
fn decode(value: &str) -> Option<String> {
    let mut bytes = value.bytes();
    let mut decoded = Vec::with_capacity(value.len());
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_forms_have_one_identity_and_no_extra_parameters() {
        let expected = "https://www.youtube.com/watch?v=AbC_dEf-123";
        for url in [
            "https://youtu.be/AbC_dEf-123?si=share&t=42#chapter",
            "http://www.youtu.be/AbC_dEf-123/",
            "HTTPS://WWW.YOUTUBE.COM/WATCH?feature=share&V=AbC_dEf-123&list=PL123&index=2",
            "https://m.youtube.com/watch?v=AbC_dEf-123&t=60",
            "https://music.youtube.com/watch?v=AbC_dEf-123&list=RD123",
            "https://gaming.youtube.com/watch?v=AbC_dEf-123",
            "https://youtube.com/shorts/AbC_dEf-123?feature=share",
            "https://youtube.com/live/AbC_dEf-123?si=share",
            "https://www.youtube-nocookie.com/embed/AbC_dEf-123?start=10&autoplay=1",
            "https://youtube.com/v/AbC_dEf-123?version=3",
            "https://youtube.com/e/AbC_dEf-123",
            "https://youtube.googleapis.com/v/AbC_dEf-123",
            "https://youtube.com/watch/v/AbC_dEf-123",
            "https://youtube.com:443/watch?%76=%41bC_dEf-123",
            "https://youtube.com./%65mbed/AbC_dEf-123",
            "www.youtube.com/watch?v=AbC_dEf-123",
            "youtu.be/AbC_dEf-123",
            "youtu.be/AbC_dEf-123?next=https://example.test",
            "//youtube.com/watch?v=AbC_dEf-123",
            "https://youtube.com/attribution_link?a=token&u=%2Fwatch%3Fv%3DAbC_dEf-123%26feature%3Dshare",
            "https://youtube.com/redirect?q=https%3A%2F%2Fyoutu.be%2FAbC_dEf-123%3Ft%3D5",
            "https://youtube.com/oembed?url=https%3A%2F%2Fyoutube.com%2Fwatch%3Fv%3DAbC_dEf-123&format=json",
            "https://youtube.com/#!/watch?v=AbC_dEf-123",
        ] {
            assert_eq!(
                link(url),
                (expected.to_lowercase(), expected.into()),
                "{url}"
            );
        }
        assert_eq!(
            link("https://youtu.be/abc_def-123").0,
            expected.to_lowercase()
        );
    }

    #[test]
    fn unrelated_and_unresolved_links_are_preserved() {
        for url in [
            "https://youtube.com/playlist?list=PL123",
            "https://youtube.com/@Creator",
            "https://youtube.com/clip/AbC_dEf-123",
            "https://youtube.com/embed/videoseries?list=PL123",
            "https://youtube.com/watch?v=too-short",
            "https://youtube.com/watch?v=AbC_dEf-1234",
            "https://youtube.com/watch?v=AbC_dEf%ZZ23",
            "https://youtube.com/watch?v=AbC_dEf%FF23",
            "https://youtube.com/shorts/AbC_dEf-123/more",
            "https://youtube.com.evil.test/watch?v=AbC_dEf-123",
            "https://notyoutube.com/watch?v=AbC_dEf-123",
            "https://youtube.com@evil.test/watch?v=AbC_dEf-123",
            "https://evil.test@www.youtube.com/watch?v=AbC_dEf-123",
            "https://evil.test/redirect?url=https%3A%2F%2Fyoutu.be%2FAbC_dEf-123",
            "ftp://youtube.com/watch?v=AbC_dEf-123",
            "https://EXAMPLE.test/Path?Token=Mixed#Section",
            "https://example.test/École",
        ] {
            assert_eq!(link(url), (url.to_lowercase(), url.into()), "{url}");
        }
        assert_ne!(
            link("https://youtu.be/AbC_dEf-123").0,
            link("https://youtu.be/XyZ_dEf-123").0
        );
    }
}
