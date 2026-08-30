use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use super::model::{ProcessInfo, Runtime, WorkloadIdentity};

const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub behavior: Behavior,
    pub excludes: Vec<ExcludeRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            behavior: Behavior::default(),
            excludes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Behavior {
    pub refresh_ms: u64,
    pub grace_period_ms: u64,
    pub confirm_kill_all: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            refresh_ms: 900,
            grace_period_ms: 1_200,
            confirm_kill_all: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleScope {
    Workload,
    Project,
    Executable,
    Runtime,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExcludeRule {
    pub id: String,
    pub name: String,
    pub scope: RuleScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    pub created_at: u64,
}

impl ExcludeRule {
    pub fn for_workload(process: &ProcessInfo) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let name = if process.project_name.is_empty() {
            process.workload_label.clone()
        } else {
            format!("{} / {}", process.project_name, process.workload_label)
        };

        Self {
            id: format!("{}-{}", created_at, process.pid),
            name,
            scope: RuleScope::Workload,
            runtime: Some(process.runtime),
            project: process.identity.anchor.clone(),
            workload: Some(process.identity.workload.clone()),
            // A recognized global entrypoint is stable across working
            // directories and runtime upgrades. The executable is only needed
            // to distinguish otherwise anonymous/interactive processes.
            executable: (process.identity.anchor.is_none()
                && process.identity.workload == "interactive")
                .then(|| process.identity.executable.clone())
                .flatten(),
            created_at,
        }
    }

    pub fn matches(&self, identity: &WorkloadIdentity) -> bool {
        if self
            .runtime
            .is_some_and(|runtime| runtime != identity.runtime)
        {
            return false;
        }

        match self.scope {
            RuleScope::Workload => {
                option_path_matches(self.project.as_deref(), identity.anchor.as_deref())
                    && option_text_matches(self.workload.as_deref(), Some(&identity.workload))
                    && option_path_matches(
                        self.executable.as_deref(),
                        identity.executable.as_deref(),
                    )
                    && (self.project.is_some()
                        || self.workload.is_some()
                        || self.executable.is_some())
            }
            RuleScope::Project => {
                self.project.is_some()
                    && option_path_matches(self.project.as_deref(), identity.anchor.as_deref())
            }
            RuleScope::Executable => {
                self.executable.is_some()
                    && option_path_matches(
                        self.executable.as_deref(),
                        identity.executable.as_deref(),
                    )
            }
            RuleScope::Runtime => self.runtime.is_some(),
        }
    }
}

fn option_text_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_none_or(|expected| actual.is_some_and(|actual| expected == actual))
}

fn option_path_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_none_or(|expected| {
        actual.is_some_and(|actual| normalize_rule_path(expected) == normalize_rule_path(actual))
    })
}

fn normalize_rule_path(path: &str) -> String {
    let normalized = path.replace('\\', "/").trim_end_matches('/').to_owned();
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

pub struct ConfigStore {
    path: PathBuf,
    config: Config,
}

impl ConfigStore {
    pub fn load() -> Result<Self> {
        Self::load_from(config_path()?)
    }

    pub fn load_from(path: PathBuf) -> Result<Self> {
        let config = if path.exists() {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let parsed: Config = toml::from_str(&source)
                .with_context(|| format!("invalid configuration in {}", path.display()))?;
            if parsed.version > CONFIG_VERSION {
                return Err(anyhow!(
                    "{} uses config version {}, but this build supports version {}",
                    path.display(),
                    parsed.version,
                    CONFIG_VERSION
                ));
            }
            parsed
        } else {
            Config::default()
        };

        Ok(Self { path, config })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn matching_rule(&self, identity: &WorkloadIdentity) -> Option<&ExcludeRule> {
        self.config
            .excludes
            .iter()
            .find(|rule| rule.matches(identity))
    }

    pub fn add_workload(&mut self, process: &ProcessInfo) -> Result<String> {
        let rule = ExcludeRule::for_workload(process);
        let label = rule.name.clone();
        self.config.excludes.push(rule);
        self.save()?;
        Ok(label)
    }

    pub fn remove_rule(&mut self, id: &str) -> Result<Option<String>> {
        let Some(index) = self.config.excludes.iter().position(|rule| rule.id == id) else {
            return Ok(None);
        };
        let removed = self.config.excludes.remove(index);
        self.save()?;
        Ok(Some(removed.name))
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let serialized = toml::to_string_pretty(&self.config)?;
        fs::write(&self.path, serialized)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

pub fn config_path() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from("", "", "bunt")
        .ok_or_else(|| anyhow!("could not determine a configuration directory"))?;
    Ok(project_dirs.config_dir().join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> WorkloadIdentity {
        WorkloadIdentity {
            runtime: Runtime::Bun,
            executable: Some("c:/tools/bun.exe".into()),
            anchor: Some("f:/project".into()),
            workload: "command:run:dev".into(),
        }
    }

    #[test]
    fn workload_rule_matches_the_same_workload_only() {
        let rule = ExcludeRule {
            id: "one".into(),
            name: "dev".into(),
            scope: RuleScope::Workload,
            runtime: Some(Runtime::Bun),
            project: Some("F:\\PROJECT\\".into()),
            workload: Some("command:run:dev".into()),
            executable: None,
            created_at: 0,
        };

        assert!(rule.matches(&identity()));

        let mut different = identity();
        different.workload = "command:run:test".into();
        assert!(!rule.matches(&different));
    }

    #[test]
    fn config_round_trips_to_disk() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bunt.toml");
        let mut store = ConfigStore::load_from(path.clone()).unwrap();
        store.config.excludes.push(ExcludeRule {
            id: "one".into(),
            name: "project".into(),
            scope: RuleScope::Project,
            runtime: None,
            project: Some("f:/project".into()),
            workload: None,
            executable: None,
            created_at: 0,
        });
        store.save().unwrap();

        let loaded = ConfigStore::load_from(path).unwrap();
        assert_eq!(loaded.config.excludes.len(), 1);
        assert!(loaded.config.excludes[0].matches(&identity()));
    }
}
