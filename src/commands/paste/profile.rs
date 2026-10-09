//! Public social-profile photos. Post URLs retain the normal media path.
use super::{MAX_PAGE_BYTES, attribute, cloudflare, free_path};
use crate::{common, progress};
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub struct Profile {
    pub site: &'static str,
    pub name: String,
    pub url: String,
}

pub fn profile(url: &str) -> Option<Profile> {
    let uri = url.parse::<ureq::http::Uri>().ok()?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return None;
    }
    let host = uri.host()?.to_ascii_lowercase();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .or_else(|| host.strip_prefix("mobile."))
        .unwrap_or(&host);
    let parts: Vec<_> = uri.path().trim_matches('/').split('/').collect();
    let (site, name) = match host {
        "x.com" | "twitter.com"
            if parts.len() == 1 || (parts.len() == 2 && parts[1] == "photo") =>
        {
            let name = parts[0];
            if name.is_empty()
                || name.len() > 15
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || [
                    "home",
                    "explore",
                    "search",
                    "i",
                    "intent",
                    "settings",
                    "notifications",
                    "messages",
                    "login",
                    "signup",
                    "share",
                    "compose",
                ]
                .contains(&name.to_ascii_lowercase().as_str())
            {
                return None;
            }
            ("x", name.to_owned())
        }
        "facebook.com" => {
            let name = if parts == ["profile.php"] {
                uri.query()?
                    .split('&')
                    .find_map(|query| query.strip_prefix("id="))?
                    .to_owned()
            } else if parts.len() == 3 && parts[0] == "people" {
                parts[2].to_owned()
            } else if parts.len() == 1
                && ![
                    "watch",
                    "reel",
                    "reels",
                    "photo.php",
                    "photos",
                    "groups",
                    "pages",
                    "events",
                    "marketplace",
                    "gaming",
                    "login",
                    "login.php",
                    "sharer.php",
                    "story.php",
                    "permalink.php",
                    "help",
                    "settings",
                    "share",
                    "privacy",
                    "policies",
                ]
                .contains(&parts[0])
            {
                parts[0].to_owned()
            } else {
                return None;
            };
            ("facebook", name)
        }
        "instagram.com"
            if parts.len() == 1
                && ![
                    "p", "reel", "reels", "stories", "explore", "accounts", "direct",
                ]
                .contains(&parts[0]) =>
        {
            ("instagram", parts[0].to_owned())
        }
        "tiktok.com" if parts.len() == 1 && parts[0].starts_with('@') => {
            ("tiktok", parts[0][1..].to_owned())
        }
        _ => return None,
    };
    if name.is_empty()
        || name.len() > 100
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return None;
    }
    Some(Profile {
        site,
        name,
        url: url.to_owned(),
    })
}

fn normalize_image(profile: &Profile, url: &str) -> Option<String> {
    let uri = url.parse::<ureq::http::Uri>().ok()?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return None;
    }
    if profile.site == "x" {
        if uri.host()? != "pbs.twimg.com" || !uri.path().starts_with("/profile_images/") {
            return None;
        }
        let mut url = url.to_owned();
        for size in ["_normal.", "_bigger.", "_mini.", "_200x200.", "_400x400."] {
            url = url.replace(size, ".");
        }
        return Some(url);
    }
    let lower = url.to_ascii_lowercase();
    if lower.contains("/rsrc.php/")
        || lower.contains("default_profile")
        || lower.contains("/static/images/")
    {
        return None;
    }
    Some(url.replace("&amp;", "&"))
}

fn candidates(profile: &Profile, html: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut add = |url: &str| {
        if let Some(url) = normalize_image(profile, url)
            && !found.contains(&url)
        {
            found.push(url);
        }
    };
    let lower = html.to_ascii_lowercase();
    let mut search = 0;
    while let Some(offset) = lower[search..].find("<meta") {
        let start = search + offset;
        let end = lower[start..]
            .find('>')
            .map_or(html.len(), |end| start + end);
        if let Some(key) = attribute(&html[start..end], "property")
            .or_else(|| attribute(&html[start..end], "name"))
            && matches!(
                key.to_ascii_lowercase().as_str(),
                "og:image:secure_url" | "og:image" | "twitter:image"
            )
            && let Some(url) = attribute(&html[start..end], "content")
        {
            add(&url);
        }
        search = end;
    }
    // Public hydration data often carries the original avatar even when OG
    // metadata is absent. Decode each JSON string rather than unescaping by hand.
    for key in [
        "profile_image_url_https",
        "profile_image_url",
        "profile_pic_url_hd",
        "profile_pic_url",
        "avatarLarger",
    ] {
        let marker = format!("\"{key}\"");
        let mut rest = html;
        while let Some(index) = rest.find(&marker) {
            rest = &rest[index + marker.len()..];
            if let Some(value) = rest.trim_start().strip_prefix(':')
                && let Some(Ok(url)) = serde_json::Deserializer::from_str(value.trim_start())
                    .into_iter::<String>()
                    .next()
            {
                add(&url);
            }
        }
    }
    found
}

fn download(
    agent: &ureq::Agent,
    profile: &Profile,
    url: &str,
    directory: &Path,
) -> Result<(PathBuf, u64), String> {
    progress::stage("Downloading the profile photo…");
    let mut response = agent
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", &profile.url)
        .call()
        .map_err(|_| "the profile photo could not be fetched")?;
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !kind.to_ascii_lowercase().starts_with("image/") {
        return Err("profile photo URL did not return an image".into());
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(32 * 1024 * 1024)
        .read_to_vec()
        .map_err(|_| "profile photo was unreadable or exceeded the size limit")?;
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader
        .format()
        .ok_or("profile photo format was not recognized")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| "profile photo was not a valid supported image")?;
    let extension = match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::WebP => "webp",
        image::ImageFormat::Gif => "gif",
        image::ImageFormat::Bmp => "bmp",
        image::ImageFormat::Tiff => "tif",
        _ => return Err("unsupported profile photo format".into()),
    };
    let name = format!(
        "{}-{}-profile.{extension}",
        profile.site,
        common::tidy_stem(&profile.name)
    );
    let mut temporary = tempfile::Builder::new()
        .prefix(".justpaste-")
        .tempfile_in(directory)
        .map_err(|e| e.to_string())?;
    temporary.write_all(&bytes).map_err(|e| e.to_string())?;
    if common::interrupted() {
        return Err("cancelled".into());
    }
    let path = free_path(directory, &name);
    temporary
        .persist_noclobber(&path)
        .map_err(|e| e.to_string())?;
    Ok((path, bytes.len() as u64))
}

pub fn paste(
    agent: &ureq::Agent,
    cloudflare: &mut cloudflare::Client,
    profile: &Profile,
    directory: &Path,
) -> Result<(PathBuf, u64), String> {
    progress::stage("Reading the social profile…");
    let page_url = std::env::var("JUSTTOOLS_PROFILE_TEST_PAGE")
        .ok()
        .filter(|url| {
            url.parse::<ureq::http::Uri>()
                .ok()
                .is_some_and(|uri| matches!(uri.host(), Some("localhost" | "127.0.0.1" | "[::1]")))
        })
        .unwrap_or_else(|| profile.url.clone());
    let html = agent
        .get(&page_url)
        .header("User-Agent", "Mozilla/5.0")
        .call()
        .ok()
        .and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(MAX_PAGE_BYTES)
                .read_to_string()
                .ok()
        });
    let mut photos = html
        .as_deref()
        .map(|html| candidates(profile, html))
        .unwrap_or_default();
    // Keep the exact supplied thumbnail as a fallback if an original-size X
    // image has been removed or the site does not publish that size.
    if let Some(html) = &html
        && profile.site == "x"
    {
        for photo in super::declared_media(html, &profile.url) {
            if photo.contains("pbs.twimg.com/profile_images/") && !photos.contains(&photo) {
                photos.push(photo);
            }
        }
    }
    for photo in photos {
        if let Ok(saved) = download(agent, profile, &photo, directory) {
            return Ok(saved);
        }
    }
    if common::interrupted() {
        return Err("cancelled".into());
    }
    if cloudflare.enabled() {
        let rendered = cloudflare.content(&profile.url)?;
        for photo in candidates(profile, &rendered) {
            if let Ok(saved) = download(agent, profile, &photo, directory) {
                return Ok(saved);
            }
        }
    }
    Err("no public profile photo was exposed at that URL; private/login-only profiles are not downloaded".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_roots_are_detected_while_posts_and_site_routes_are_kept() {
        for url in [
            "https://x.com/Cloudflare",
            "https://twitter.com/user/photo",
            "https://www.facebook.com/cloudflare",
            "https://facebook.com/profile.php?id=123456",
            "https://facebook.com/people/Name/123456",
            "https://instagram.com/user.name",
            "https://tiktok.com/@user",
        ] {
            assert!(profile(url).is_some(), "{url}");
        }
        for url in [
            "https://x.com/user/status/123",
            "https://x.com/home",
            "https://facebook.com/watch?v=1",
            "https://facebook.com/user/videos/123",
            "https://instagram.com/p/abc",
            "https://tiktok.com/@user/video/123",
            "https://x.com.evil.test/user",
        ] {
            assert!(profile(url).is_none(), "{url}");
        }
    }
    #[test]
    fn x_photo_uses_the_avatar_not_the_profile_banner() {
        let profile = profile("https://x.com/user").unwrap();
        let html = r#"<meta name="twitter:image" content="https://pbs.twimg.com/profile_banners/1/2"><meta property="og:image" content="https://pbs.twimg.com/profile_images/1/avatar_200x200.jpg">"#;
        assert_eq!(
            candidates(&profile, html),
            ["https://pbs.twimg.com/profile_images/1/avatar.jpg"]
        );
        assert_eq!(
            candidates(
                &profile,
                r#"{"profile_image_url_https":"https:\/\/pbs.twimg.com\/profile_images\/1\/photo_normal.png"}"#
            ),
            ["https://pbs.twimg.com/profile_images/1/photo.png"]
        );
    }
}
