use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Node,
    Bun,
    Python,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Bun => "bun",
            Self::Python => "python",
        }
    }
}

impl fmt::Display for Runtime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkloadIdentity {
    pub runtime: Runtime,
    pub executable: Option<String>,
    pub anchor: Option<String>,
    pub workload: String,
}

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub runtime: Runtime,
    pub process_name: String,
    pub executable: Option<String>,
    pub cwd: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub disk_read_bytes: u64,
    pub disk_written_bytes: u64,
    pub start_time: u64,
    pub run_time: u64,
    pub status: String,
    pub project_name: String,
    pub project_root: Option<String>,
    pub workload_label: String,
    pub identity: WorkloadIdentity,
}

impl ProcessInfo {
    pub fn key(&self) -> (u32, u64) {
        (self.pid, self.start_time)
    }

    pub fn kill_target(&self) -> KillTarget {
        KillTarget {
            pid: self.pid,
            start_time: self.start_time,
            runtime: self.runtime,
            workload: self.identity.workload.clone(),
        }
    }

    pub fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {} {} {}",
            self.runtime,
            self.pid,
            self.process_name,
            self.project_name,
            self.project_root.as_deref().unwrap_or_default(),
            self.cwd.as_deref().unwrap_or_default(),
            self.workload_label,
            self.command,
        )
        .to_lowercase()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KillTarget {
    pub pid: u32,
    pub start_time: u64,
    pub runtime: Runtime,
    pub workload: String,
}
