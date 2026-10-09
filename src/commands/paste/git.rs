//! Repository links use Git directly, before any HTTP or media download.
use super::{TOOL, percent_decode};
use crate::{commands::download, common, deps, progress};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, PartialEq, Eq)]
pub struct Repository {
    pub remote: String,
    pub name: String,
}

/// Recognize explicit Git transports and clone URLs, plus repository homepages.
/// Deeper file, issue, release, and archive pages keep the normal download path.
pub fn repository(link: &str) -> Option<Repository> {
    if link.chars().any(|c| c.is_whitespace() || c.is_control()) || link.contains('\\') {
        return None;
    }
    let (remote, path, explicit) = if link.contains("://") {
        let (scheme, rest) = link.split_once("://")?;
        let remote = format!(
            "{}://{}",
            scheme.to_ascii_lowercase(),
            rest.split(['?', '#']).next()?.trim_end_matches('/')
        );
        let uri = remote.parse::<ureq::http::Uri>().ok()?;
        let scheme = uri.scheme_str()?.to_ascii_lowercase();
        let host = uri.host()?.to_ascii_lowercase();
        if host.starts_with('-') {
            return None;
        }
        let path = uri.path().trim_matches('/');
        let explicit = match scheme.as_str() {
            "ssh" | "git" => true,
            "http" | "https" => {
                let parts: Vec<_> = path.split('/').collect();
                let host = host.strip_prefix("www.").unwrap_or(&host);
                let homepage = match host {
                    "github.com" => {
                        parts.len() == 2
                            && !matches!(
                                parts[0],
                                "orgs"
                                    | "users"
                                    | "topics"
                                    | "collections"
                                    | "marketplace"
                                    | "settings"
                                    | "features"
                                    | "sponsors"
                                    | "enterprise"
                            )
                    }
                    "bitbucket.org" | "codeberg.org" | "gitea.com" | "git.sr.ht" => {
                        parts.len() == 2
                            && !matches!(
                                parts[0],
                                "account" | "product" | "site" | "workspaces" | "explore" | "user"
                            )
                    }
                    "gitlab.com" => {
                        parts.len() >= 2
                            && !matches!(
                                parts[0],
                                "users"
                                    | "groups"
                                    | "explore"
                                    | "help"
                                    | "admin"
                                    | "dashboard"
                                    | "profile"
                                    | "search"
                                    | "oauth"
                                    | "projects"
                            )
                            && !parts.iter().any(|part| {
                                matches!(
                                    *part,
                                    "-" | "blob"
                                        | "tree"
                                        | "raw"
                                        | "issues"
                                        | "merge_requests"
                                        | "releases"
                                        | "archive"
                                        | "wikis"
                                        | "commits"
                                        | "tags"
                                )
                            })
                    }
                    "dev.azure.com" => parts.len() == 4 && parts[2] == "_git",
                    _ if host.ends_with(".visualstudio.com") => {
                        parts.len() == 3 && parts[1] == "_git"
                    }
                    _ => false,
                };
                let known_host = matches!(
                    host,
                    "github.com"
                        | "gitlab.com"
                        | "bitbucket.org"
                        | "codeberg.org"
                        | "gitea.com"
                        | "git.sr.ht"
                        | "dev.azure.com"
                ) || host.ends_with(".visualstudio.com");
                homepage || (!known_host && path.ends_with(".git"))
            }
            _ => return None,
        };
        (remote, path.to_owned(), explicit)
    } else {
        // Require a user@host or a dotted host to avoid treating drive letters
        // and ordinary local paths as scp-style SSH remotes.
        let (authority, path) = link.split_once(':')?;
        let (user, host) = authority.rsplit_once('@').unwrap_or(("", authority));
        if host.is_empty()
            || host.starts_with('-')
            || authority.contains('/')
            || (!authority.contains('@') && !host.contains('.'))
            || !host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            || (!user.is_empty()
                && !user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        {
            return None;
        }
        (
            link.trim_end_matches('/').to_owned(),
            path.trim_matches('/').to_owned(),
            true,
        )
    };
    if !explicit
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return None;
    }
    let segment = path.rsplit('/').next()?;
    let segment = segment.strip_suffix(".git").unwrap_or(segment);
    let name = common::tidy_stem(&percent_decode(segment));
    if name.is_empty() {
        return None;
    }
    Some(Repository { remote, name })
}

/// Dots belong to repository folder names, not file extensions.
pub fn destination(directory: &Path, name: &str) -> PathBuf {
    (1_u32..)
        .map(|number| {
            directory.join(if number == 1 {
                name.to_owned()
            } else {
                format!("{name} ({number})")
            })
        })
        .find(|path| fs::symlink_metadata(path).is_err())
        .expect("an unused number always exists")
}

fn progress_line(line: &str, reporter: &progress::Reporter) {
    let line = line.trim();
    let percent = line.split_once('%').and_then(|(before, _)| {
        before
            .rsplit_once([' ', ':'])
            .and_then(|(_, value)| value.parse::<u32>().ok())
    });
    reporter.report("Cloning Git repository…", line, percent);
}

pub fn clone(repository: &Repository, directory: &Path) -> Result<PathBuf, String> {
    progress::stage("Preparing Git…");
    let override_path = std::env::var_os("GIT_BIN").filter(|value| !value.is_empty());
    let available = match &override_path {
        Some(path) => {
            Path::new(path).is_file() || path.to_str().and_then(deps::find_executable).is_some()
        }
        None => deps::find_executable("git").is_some(),
    };
    if !available {
        if override_path.is_some() {
            eprintln!(
                "{TOOL}: warning: GIT_BIN does not point to an installed Git executable. Set GIT_BIN to Git's path, or remove it to use PATH."
            );
        } else {
            eprintln!(
                "{TOOL}: warning: Git is not installed. Repository links need Git; install it from https://git-scm.com/downloads or your system package manager. Nothing will be cloned until Git is available."
            );
        }
    }
    let git = deps::require_override(TOOL, "GIT_BIN", "git")
        .map_err(|error| error.message().to_owned())?;
    let git = git.canonicalize().unwrap_or(git);
    if common::interrupted() {
        return Err("cancelled".into());
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let workspace = tempfile::Builder::new()
        .prefix(".justpaste-")
        .tempdir_in(&directory)
        .map_err(|e| format!("cannot write in {}: {e}", directory.display()))?;
    let checkout = workspace.path().join("checkout");
    let mut command = Command::new(git);
    command
        // A relative worktree argument avoids passing Windows verbatim paths
        // (\\?\...) to Git for Windows, which does not accept them here.
        .current_dir(workspace.path())
        .args(["clone", "--progress", "--"])
        .arg(&repository.remote)
        .arg("checkout")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never");
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(variable);
    }
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
    }
    download::hide_console(&mut command);
    progress::stage("Cloning Git repository…");
    let (status, detail) =
        download::capture_with_progress(&mut command, None, true, progress_line)?;
    if !status.success() {
        let reason = detail
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("Git exited without completing the clone");
        return Err(format!("Git could not clone the repository: {reason}"));
    }
    if common::interrupted() {
        return Err("cancelled".into());
    }
    if !checkout.join(".git").is_dir() {
        return Err("Git reported success but did not create a working repository".into());
    }
    progress::stage("Saving Git repository…");
    let target = destination(&directory, &repository.name);
    common::rename_directory_noclobber(&checkout, &target)
        .map_err(|e| format!("cannot save {}: {e}", target.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_roots_and_explicit_clone_urls_are_recognized() {
        for (link, name) in [
            ("https://github.com/owner/repo", "repo"),
            ("HTTPS://github.com/owner/repo", "repo"),
            (
                "https://GitHub.com/owner/repo.git/?tab=readme#intro",
                "repo",
            ),
            ("https://gitlab.com/group/subgroup/repo", "repo"),
            ("https://bitbucket.org/team/repo", "repo"),
            ("https://codeberg.org/team/repo", "repo"),
            ("https://git.sr.ht/~owner/repo", "repo"),
            ("https://dev.azure.com/org/project/_git/repo", "repo"),
            ("https://org.visualstudio.com/project/_git/repo", "repo"),
            ("https://forge.example/team/repo.name.git", "repo.name"),
            ("ssh://git@forge.example:2222/team/repo.git", "repo"),
            ("git://forge.example/team/repo", "repo"),
            ("git@github.com:owner/repo.git", "repo"),
            ("github.com:owner/repo", "repo"),
            ("https://forge.example/team/CON.git", "_CON"),
            ("https://forge.example/team/..%2Frepo.git", "repo"),
        ] {
            assert_eq!(repository(link).unwrap().name, name, "{link}");
        }
        assert_eq!(
            repository("https://github.com/owner/repo/?tab=readme#intro")
                .unwrap()
                .remote,
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn downloads_and_non_repository_pages_stay_out_of_git() {
        for link in [
            "https://github.com/owner",
            "https://github.com/orgs/owner",
            "https://github.com/owner/repo/blob/main/file.rs",
            "https://github.com/owner/repo/blob/main/file.git",
            "https://github.com/owner/repo/issues/1",
            "https://github.com/owner/repo/archive/refs/heads/main.zip",
            "https://gitlab.com/group/repo/-/tree/main",
            "https://gitlab.com/group/repo/issues/1",
            "https://bitbucket.org/team/repo/src/main/file.rs",
            "https://gitlab.com/group/repo/-/raw/main/file.git",
            "https://example.com/photo.png",
            "https://github.com.evil.test/owner/repo",
            "https://example.com/team/repo.git/archive.zip",
            "file:///tmp/repo.git",
            "ext::some-command",
            "F:\\repo.git",
            "https://example.com/../repo.git",
            "ssh://-bad/../repo.git",
            "git@github.com:owner/..",
            "git@github.com:",
            "ftp://example.com/repo.git",
        ] {
            assert!(repository(link).is_none(), "{link}");
        }
    }

    #[test]
    fn folder_collisions_keep_the_complete_repo_name() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("repo.name")).unwrap();
        fs::write(directory.path().join("repo.name (2)"), "kept").unwrap();
        assert_eq!(
            destination(directory.path(), "repo.name"),
            directory.path().join("repo.name (3)")
        );
    }
}
