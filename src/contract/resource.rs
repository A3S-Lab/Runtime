use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    Process,
    Container,
    Sandbox,
    Confidential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    pub cpu_millis: u64,
    pub memory_bytes: u64,
    pub pids: u32,
    pub ephemeral_storage_bytes: u64,
    /// Required for finite Tasks and forbidden for long-running Services.
    pub execution_timeout_ms: Option<u64>,
}

impl ResourceLimits {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.cpu_millis == 0
            || self.memory_bytes == 0
            || self.pids == 0
            || self.ephemeral_storage_bytes == 0
        {
            return Err("all Runtime resource limits must be positive".into());
        }
        if self.execution_timeout_ms == Some(0) {
            return Err("execution_timeout_ms must be positive when present".into());
        }
        Ok(())
    }
}
