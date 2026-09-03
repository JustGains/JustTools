use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRecipe {
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
}

impl LaunchRecipe {
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub port: u16,
    pub pid: u32,
    pub url: String,
    pub addresses: Vec<String>,
    pub project_name: String,
    pub project_root: Option<String>,
    pub framework: String,
    pub process_name: String,
    pub command: String,
    pub cwd: Option<String>,
    pub executable: Option<String>,
    pub run_time_seconds: u64,
    pub start_time: u64,
    pub memory_bytes: u64,
    pub is_dev_server: bool,
    pub detection_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchRecipe>,
}

impl ServerInfo {
    pub fn key(&self) -> (u16, u32) {
        (self.port, self.pid)
    }

    pub fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {} {} {}",
            self.port,
            self.url,
            self.project_name,
            self.framework,
            self.process_name,
            self.command,
            self.cwd.as_deref().unwrap_or_default(),
            self.project_root.as_deref().unwrap_or_default(),
        )
        .to_lowercase()
    }
}
