use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use super::model::{LaunchRecipe, ServerInfo};

const CACHE_VERSION: u32 = 1;
const MAX_HISTORY: usize = 40;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownServer {
    pub project_name: String,
    pub project_root: Option<String>,
    pub url: String,
    pub port: u16,
    pub framework: String,
    pub last_seen: u64,
    pub launch: Option<LaunchRecipe>,
}

impl KnownServer {
    pub fn key(&self) -> String {
        history_key(self.project_root.as_deref(), &self.url, self.port)
    }

    pub fn launch_label(&self) -> &str {
        self.launch
            .as_ref()
            .map(|launch| launch.label.as_str())
            .unwrap_or("no start command detected")
    }

    pub fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.port,
            self.url,
            self.project_name,
            self.project_root.as_deref().unwrap_or_default(),
            self.framework,
            self.launch_label(),
        )
        .to_lowercase()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct CacheFile {
    version: u32,
    servers: Vec<KnownServer>,
}

impl Default for CacheFile {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            servers: Vec::new(),
        }
    }
}

pub struct HistoryStore {
    path: PathBuf,
    cache: CacheFile,
}

impl HistoryStore {
    pub fn load() -> Result<Self> {
        Self::load_from(cache_path()?)
    }

    fn load_from(path: PathBuf) -> Result<Self> {
        let cache = if path.exists() {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let parsed: CacheFile = serde_json::from_str(&source)
                .with_context(|| format!("invalid JustPorts history in {}", path.display()))?;
            if parsed.version > CACHE_VERSION {
                return Err(anyhow!(
                    "{} uses cache version {}, but this build supports version {}",
                    path.display(),
                    parsed.version,
                    CACHE_VERSION
                ));
            }
            parsed
        } else {
            CacheFile::default()
        };
        Ok(Self { path, cache })
    }

    pub fn record(&mut self, servers: &[ServerInfo]) -> Result<()> {
        let now = now_epoch();
        let mut changed = false;
        let before = self.cache.servers.len();
        self.cache.servers.retain(|known| {
            !servers
                .iter()
                .any(|server| !server.is_dev_server && server.port == known.port)
        });
        changed |= self.cache.servers.len() != before;
        for server in servers.iter().filter(|server| server.is_dev_server) {
            let key = history_key(server.project_root.as_deref(), &server.url, server.port);
            let before = self.cache.servers.len();
            self.cache.servers.retain(|known| {
                known.key() == key
                    || known.port != server.port
                    || (known.project_name != server.project_name
                        && known.url != server.url
                        && !related_roots(
                            known.project_root.as_deref(),
                            server.project_root.as_deref(),
                        ))
            });
            changed |= self.cache.servers.len() != before;
            let known = KnownServer {
                project_name: server.project_name.clone(),
                project_root: server.project_root.clone(),
                url: server.url.clone(),
                port: server.port,
                framework: server.framework.clone(),
                last_seen: now,
                launch: server.launch.clone(),
            };
            if let Some(existing) = self
                .cache
                .servers
                .iter_mut()
                .find(|candidate| candidate.key() == key)
            {
                if existing.url != known.url
                    || existing.port != known.port
                    || existing.framework != known.framework
                    || existing.launch.as_ref().map(LaunchRecipe::display)
                        != known.launch.as_ref().map(LaunchRecipe::display)
                    || now.saturating_sub(existing.last_seen) >= 60
                {
                    *existing = known;
                    changed = true;
                }
            } else {
                self.cache.servers.push(known);
                changed = true;
            }
        }
        self.cache
            .servers
            .sort_by_key(|server| std::cmp::Reverse(server.last_seen));
        if self.cache.servers.len() > MAX_HISTORY {
            self.cache.servers.truncate(MAX_HISTORY);
            changed = true;
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    pub fn offline(&self, active: &[ServerInfo]) -> Vec<KnownServer> {
        self.cache
            .servers
            .iter()
            .filter(|known| {
                !active.iter().any(|server| {
                    server.port == known.port
                        || history_key(server.project_root.as_deref(), &server.url, server.port)
                            == known.key()
                })
            })
            .cloned()
            .collect()
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let encoded = serde_json::to_vec_pretty(&self.cache)?;
        let mut output = AtomicWriteFile::open(&self.path)
            .with_context(|| format!("failed to stage {}", self.path.display()))?;
        output
            .write_all(&encoded)
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        output
            .commit()
            .with_context(|| format!("failed to save {}", self.path.display()))
    }
}

pub fn cache_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("JUSTPORTS_HISTORY").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let project_dirs = ProjectDirs::from("", "", "JustTools")
        .ok_or_else(|| anyhow!("could not determine a JustTools data directory"))?;
    Ok(project_dirs.data_local_dir().join("ports-history.json"))
}

fn history_key(project_root: Option<&str>, url: &str, port: u16) -> String {
    let key = project_root
        .map(|root| format!("{}|{port}", root.replace('\\', "/")))
        .unwrap_or_else(|| url.to_owned());
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

fn related_roots(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    let normalize = |value: &str| {
        let value = value.replace('\\', "/").trim_end_matches('/').to_owned();
        if cfg!(windows) {
            value.to_lowercase()
        } else {
            value
        }
    };
    let left = normalize(left);
    let right = normalize(right);
    left == right
        || left
            .strip_prefix(&right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(&left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(root: &str, port: u16) -> ServerInfo {
        ServerInfo {
            port,
            pid: 42,
            url: format!("http://localhost:{port}/"),
            addresses: vec!["127.0.0.1".into()],
            project_name: "demo".into(),
            project_root: Some(root.into()),
            framework: "Vite".into(),
            process_name: "node".into(),
            command: "vite".into(),
            cwd: Some(root.into()),
            executable: None,
            run_time_seconds: 1,
            start_time: 1,
            memory_bytes: 1,
            is_dev_server: true,
            detection_reason: "test".into(),
            launch: Some(LaunchRecipe {
                label: "npm run dev".into(),
                program: "npm".into(),
                args: vec!["run".into(), "dev".into()],
                cwd: root.into(),
            }),
        }
    }

    #[test]
    fn history_round_trips_and_deduplicates_a_project_port() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.json");
        let mut store = HistoryStore::load_from(path.clone()).unwrap();
        store.record(&[server("F:/demo", 3000)]).unwrap();
        store.record(&[server("F:/demo", 3000)]).unwrap();

        let loaded = HistoryStore::load_from(path).unwrap();
        assert_eq!(loaded.cache.servers.len(), 1);
        assert_eq!(loaded.cache.servers[0].port, 3000);
        assert!(loaded.offline(&[]).first().unwrap().launch.is_some());
    }

    #[test]
    fn active_projects_are_hidden_from_offline_history() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::load_from(directory.path().join("history.json")).unwrap();
        let active = server("F:/demo", 3000);
        store.record(std::slice::from_ref(&active)).unwrap();
        assert!(store.offline(&[active]).is_empty());
    }
}
