use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
struct Preferences {
    version: u32,
    tools: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: VERSION,
            tools: BTreeMap::new(),
        }
    }
}

pub struct Store {
    path: PathBuf,
    preferences: Preferences,
}

impl Store {
    pub fn load() -> Result<Self> {
        Self::load_from(path()?)
    }

    pub fn load_from(path: PathBuf) -> Result<Self> {
        let preferences = if path.exists() {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let parsed: Preferences = toml::from_str(&source)
                .with_context(|| format!("invalid JustTools defaults in {}", path.display()))?;
            if parsed.version > VERSION {
                return Err(anyhow!(
                    "{} uses defaults version {}, but this build supports version {}",
                    path.display(),
                    parsed.version,
                    VERSION
                ));
            }
            parsed
        } else {
            Preferences::default()
        };
        Ok(Self { path, preferences })
    }

    pub fn get(&self, tool: &str, field: &str) -> Option<&str> {
        self.preferences
            .tools
            .get(tool)?
            .get(field)
            .map(String::as_str)
    }

    pub fn set(&mut self, tool: &str, field: &str, value: Option<&str>) -> Result<()> {
        self.preferences = Self::load_from(self.path.clone())?.preferences;
        if let Some(value) = value {
            self.preferences
                .tools
                .entry(tool.to_owned())
                .or_default()
                .insert(field.to_owned(), value.to_owned());
        } else if let Some(fields) = self.preferences.tools.get_mut(tool) {
            fields.remove(field);
            if fields.is_empty() {
                self.preferences.tools.remove(tool);
            }
        }
        self.save()
    }

    pub fn reset_tool(&mut self, tool: &str) -> Result<()> {
        self.preferences = Self::load_from(self.path.clone())?.preferences;
        self.preferences.tools.remove(tool);
        self.save()
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let encoded = toml::to_string_pretty(&self.preferences)?;
        let mut output = AtomicWriteFile::open(&self.path)
            .with_context(|| format!("failed to stage {}", self.path.display()))?;
        output
            .write_all(encoded.as_bytes())
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        output
            .commit()
            .with_context(|| format!("failed to save {}", self.path.display()))
    }

    pub fn file_path(&self) -> &Path {
        &self.path
    }
}

pub fn path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("JUSTTOOLS_DEFAULTS").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let dirs = ProjectDirs::from("", "", "JustTools")
        .ok_or_else(|| anyhow!("could not determine a JustTools configuration directory"))?;
    Ok(dirs.config_dir().join("defaults.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_and_builtin_values_are_removable() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("defaults.toml");
        let mut store = Store::load_from(path.clone()).unwrap();
        store.set("justjpg", "quality", Some("92")).unwrap();
        assert_eq!(
            Store::load_from(path.clone())
                .unwrap()
                .get("justjpg", "quality"),
            Some("92")
        );
        store.set("justjpg", "quality", None).unwrap();
        assert_eq!(
            Store::load_from(path).unwrap().get("justjpg", "quality"),
            None
        );
    }

    #[test]
    fn independent_open_launchers_merge_the_latest_saved_tools() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("defaults.toml");
        let mut first = Store::load_from(path.clone()).unwrap();
        let mut second = Store::load_from(path.clone()).unwrap();
        first.set("justjpg", "quality", Some("90")).unwrap();
        second.set("justzip", "compression", Some("fast")).unwrap();
        let loaded = Store::load_from(path).unwrap();
        assert_eq!(loaded.get("justjpg", "quality"), Some("90"));
        assert_eq!(loaded.get("justzip", "compression"), Some("fast"));
    }
}
