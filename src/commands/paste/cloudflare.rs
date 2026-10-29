//! Optional Browser Run requests. Credentials stay in memory and out of errors.
use super::{MAX_PAGE_BYTES, free_path};
use crate::{common, progress};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

const API: &str = "https://api.cloudflare.com/client/v4";

pub(super) fn environment(name: &str) -> Option<String> {
    // An explicit empty process value disables a persisted Windows value,
    // which also makes tests and unattended overrides deterministic.
    match std::env::var_os(name) {
        Some(value) => value
            .into_string()
            .ok()
            .filter(|value| !value.trim().is_empty()),
        None => {
            #[cfg(windows)]
            {
                use winreg::{
                    RegKey,
                    enums::{HKEY_CURRENT_USER, KEY_READ},
                };
                RegKey::predef(HKEY_CURRENT_USER)
                    .open_subkey_with_flags("Environment", KEY_READ)
                    .ok()?
                    .get_value::<String, _>(name)
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            }
            #[cfg(not(windows))]
            {
                None
            }
        }
    }
}

pub struct Client {
    key: Option<String>,
    email: Option<String>,
    account: Option<String>,
    resolved: Option<Result<String, String>>,
    base: String,
}

impl Client {
    pub fn new() -> Self {
        let token = environment("CLOUDFLARE_API_TOKEN");
        let email = if token.is_some() {
            None
        } else {
            environment("CLOUDFLARE_EMAIL")
        };
        Self {
            key: token.or_else(|| environment("CLOUDFLARE_API_KEY")),
            email,
            account: environment("CLOUDFLARE_ACCOUNT_ID").or_else(|| environment("CF_ACCOUNT_ID")),
            resolved: None,
            base: std::env::var("JUSTTOOLS_CLOUDFLARE_TEST_BASE").unwrap_or_else(|_| API.into()),
        }
    }
    pub fn enabled(&self) -> bool {
        self.key.is_some()
    }

    fn request(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let key = self.key.as_deref().ok_or("Markdown needs CLOUDFLARE_API_KEY (or CLOUDFLARE_API_TOKEN) with Browser Rendering - Edit permission")?;
        if self.base != API {
            let uri = self
                .base
                .parse::<ureq::http::Uri>()
                .map_err(|_| "invalid Cloudflare test endpoint")?;
            if !matches!(uri.host(), Some("localhost" | "127.0.0.1" | "[::1]")) {
                return Err("Cloudflare test endpoints must be loopback; credentials are sent only to Cloudflare in normal use".into());
            }
        }
        if common::interrupted() {
            return Err("cancelled".into());
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_global(Some(Duration::from_secs(90)))
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!("{}{}", self.base.trim_end_matches('/'), path);
        macro_rules! auth {
            ($request:expr) => {{
                let request = $request.header("Accept", "application/json");
                if let Some(email) = &self.email {
                    request
                        .header("X-Auth-Key", key)
                        .header("X-Auth-Email", email)
                } else {
                    request.header("Authorization", format!("Bearer {}", key.trim()))
                }
            }};
        }
        let response = match body {
            Some(body) => auth!(agent.post(&url))
                .header("Content-Type", "application/json")
                .send(serde_json::to_vec(&body).map_err(|_| "cannot prepare Cloudflare request")?),
            None => auth!(agent.get(&url)).call(),
        };
        let mut response = response.map_err(|_| "Cloudflare request could not be completed")?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_PAGE_BYTES * 2)
            .read_to_string()
            .map_err(|_| "Cloudflare response was unreadable or exceeded the size limit")?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| "Cloudflare returned invalid JSON")?;
        if !(200..300).contains(&status) || value["success"] != true {
            // Report numeric codes, never remote error text that could echo a
            // credential or authenticated request data.
            let codes: Vec<_> = value["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e["code"].as_i64())
                .map(|n| n.to_string())
                .collect();
            let guidance = if status == 429 {
                "Cloudflare is rate-limiting browser requests; retry after a short wait"
            } else if status >= 500 {
                "Cloudflare's browser service is temporarily unavailable; retry later"
            } else {
                "check the token's Browser Rendering - Edit permission and account access"
            };
            return Err(format!(
                "Cloudflare returned HTTP {status}{}; {guidance}",
                if codes.is_empty() {
                    String::new()
                } else {
                    format!(" (codes {})", codes.join(", "))
                }
            ));
        }
        Ok(value)
    }

    fn discover(&self) -> Result<String, String> {
        if let Some(account) = &self.account {
            if account.len() != 32 || !account.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(
                    "CLOUDFLARE_ACCOUNT_ID must be a 32-character hexadecimal account ID".into(),
                );
            }
            return Ok(account.clone());
        }
        progress::stage("Finding the Cloudflare account…");
        for endpoint in ["accounts", "zones"] {
            let mut accounts = BTreeSet::new();
            for page in 1..=20 {
                let response =
                    match self.request(&format!("/{endpoint}?per_page=50&page={page}"), None) {
                        Ok(response) => response,
                        Err(_) => break,
                    };
                let entries = response["result"]
                    .as_array()
                    .ok_or("Cloudflare account discovery returned no list")?;
                for entry in entries {
                    let id = if endpoint == "accounts" {
                        entry["id"].as_str()
                    } else {
                        entry["account"]["id"].as_str()
                    };
                    if let Some(id) = id {
                        accounts.insert(id.to_owned());
                    }
                }
                if accounts.len() > 1 {
                    return Err("the Cloudflare key sees multiple accounts; set CLOUDFLARE_ACCOUNT_ID to choose one".into());
                }
                let total = response["result_info"]["total_count"]
                    .as_u64()
                    .unwrap_or(entries.len() as u64);
                if page * 50 >= total || entries.is_empty() {
                    if let Some(account) = accounts.into_iter().next() {
                        return Ok(account);
                    }
                    break;
                }
                if page == 20 {
                    return Err(
                        "Cloudflare discovery needs an explicit CLOUDFLARE_ACCOUNT_ID".into(),
                    );
                }
            }
        }
        Err("cannot discover a Cloudflare account with this key; set CLOUDFLARE_ACCOUNT_ID, or allow account/zone read access".into())
    }

    fn extract(&mut self, action: &str, url: &str) -> Result<String, String> {
        if !self.enabled() {
            return Err(
                "set CLOUDFLARE_API_KEY (or CLOUDFLARE_API_TOKEN) to use Cloudflare Markdown"
                    .into(),
            );
        }
        let account = if let Some(resolved) = &self.resolved {
            resolved.clone()?
        } else {
            let resolved = self.discover();
            self.resolved = Some(resolved.clone());
            resolved?
        };
        progress::stage(if action == "markdown" {
            "Converting the page to Markdown…"
        } else {
            "Reading the rendered profile page…"
        });
        let response = self.request(
            &format!("/accounts/{account}/browser-run/{action}"),
            Some(serde_json::json!({"url":url,"gotoOptions":{"waitUntil":"networkidle2"}})),
        )?;
        let result = response["result"]
            .as_str()
            .ok_or("Cloudflare returned no page content")?;
        if result.trim().is_empty() {
            return Err("Cloudflare returned empty page content; no file was saved".into());
        }
        Ok(result.to_owned())
    }
    pub fn content(&mut self, url: &str) -> Result<String, String> {
        self.extract("content", url)
    }
    pub fn markdown(
        &mut self,
        url: &str,
        html: Option<&str>,
        directory: &Path,
    ) -> Result<(PathBuf, u64), String> {
        let markdown = self.extract("markdown", url)?;
        let name = markdown_name(url, html, &markdown);
        let mut temporary = tempfile::Builder::new()
            .prefix(".justpaste-")
            .tempfile_in(directory)
            .map_err(|e| e.to_string())?;
        temporary
            .write_all(markdown.as_bytes())
            .map_err(|e| e.to_string())?;
        if !markdown.ends_with('\n') {
            temporary.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        let size = temporary
            .as_file()
            .metadata()
            .map_err(|e| e.to_string())?
            .len();
        if common::interrupted() {
            return Err("cancelled".into());
        }
        let path = free_path(directory, &name);
        temporary
            .persist_noclobber(&path)
            .map_err(|e| e.to_string())?;
        Ok((path, size))
    }
}

fn markdown_name(url: &str, html: Option<&str>, markdown: &str) -> String {
    let title = html.and_then(|html| {
        let lower = html.to_ascii_lowercase();
        let start = lower.find("<title")?;
        let start = start + lower[start..].find('>')? + 1;
        let end = start + lower[start..].find("</title>")?;
        Some(
            html[start..end]
                .replace("&amp;", "&")
                .replace("&quot;", "\"")
                .replace("&#39;", "'"),
        )
    });
    let heading = markdown.lines().find_map(|line| line.strip_prefix("# "));
    let frontmatter = markdown.strip_prefix("---\n").and_then(|body| {
        body.lines()
            .take_while(|line| *line != "---")
            .take(30)
            .find_map(|line| {
                line.strip_prefix("title:")
                    .map(|title| title.trim().trim_matches(['\"', '\'']))
            })
    });
    let fallback = super::file_name(url, None, "text/markdown");
    let fallback = fallback.strip_suffix(".md").unwrap_or(&fallback);
    let stem = title
        .as_deref()
        .into_iter()
        .chain(frontmatter)
        .chain(heading)
        .chain(Some(fallback))
        .map(common::tidy_stem)
        .find(|stem| !stem.is_empty())
        .unwrap_or_else(|| "page".into());
    format!("{stem}.md")
}

/// Keep existing video/download behavior when a page clearly declares media.
pub fn is_media_page(url: &str, html: Option<&str>) -> bool {
    let host = url
        .parse::<ureq::http::Uri>()
        .ok()
        .and_then(|u| u.host().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    matches!(
        host,
        "youtube.com" | "youtu.be" | "vimeo.com" | "tiktok.com" | "twitch.tv"
    ) || ((matches!(host, "x.com" | "twitter.com") && url.contains("/status/"))
        || (host.ends_with("facebook.com")
            && (url.contains("/videos/") || url.contains("/reel/") || url.contains("/watch"))))
        || html.is_some_and(|html| {
            let lower = html.to_ascii_lowercase();
            lower.contains("<video")
                || lower.contains("<audio")
                || lower.contains("og:video")
                || lower.contains("twitter:player")
                || lower.contains("youtube.com/embed/")
                || lower.contains("player.vimeo.com/video/")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_names_use_page_title_then_heading_and_keep_a_markdown_extension() {
        assert_eq!(
            markdown_name(
                "https://example.test/article",
                Some("<title>A &amp; B</title>"),
                "# Other\n"
            ),
            "A & B.md"
        );
        assert_eq!(
            markdown_name("https://example.test/", None, "# Hello world\n"),
            "Hello world.md"
        );
        assert_eq!(
            markdown_name(
                "https://example.test/",
                None,
                "---\ntitle: \"Example Domain\"\n---\n\nBody"
            ),
            "Example Domain.md"
        );
        assert!(markdown_name("https://example.test/path", None, "body").ends_with(".md"));
        assert!(is_media_page("https://x.com/user/status/123", None));
        assert!(is_media_page(
            "https://example.test/page",
            Some("<video src='clip.mp4'>")
        ));
        assert!(!is_media_page(
            "https://developers.cloudflare.com/docs/",
            Some("<h1>Docs</h1>")
        ));
    }
}
