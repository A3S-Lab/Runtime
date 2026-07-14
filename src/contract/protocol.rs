use super::{RuntimeObservation, RuntimeUnitSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeApplyRequest {
    pub schema: String,
    pub request_id: String,
    pub deadline_at_ms: Option<u64>,
    pub spec: RuntimeUnitSpec,
}

impl RuntimeApplyRequest {
    pub const SCHEMA: &'static str = "a3s.runtime.apply-request.v1";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime apply schema {:?}",
                self.schema
            ));
        }
        super::validate_id("request_id", &self.request_id, 512)?;
        if self.deadline_at_ms == Some(0) {
            return Err("deadline_at_ms must be positive when present".into());
        }
        self.spec.validate()
    }

    pub fn digest(&self) -> Result<String, String> {
        canonical_digest(self, self.validate())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeActionRequest {
    pub schema: String,
    pub request_id: String,
    pub unit_id: String,
    pub generation: u64,
    pub deadline_at_ms: Option<u64>,
}

impl RuntimeActionRequest {
    pub const SCHEMA: &'static str = "a3s.runtime.action-request.v1";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime action schema {:?}",
                self.schema
            ));
        }
        super::validate_id("request_id", &self.request_id, 512)?;
        super::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0 || self.deadline_at_ms == Some(0) {
            return Err("action generation and deadline must be positive".into());
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, String> {
        canonical_digest(self, self.validate())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRemoval {
    pub schema: String,
    pub request_id: String,
    pub unit_id: String,
    pub generation: u64,
    pub removed_at_ms: u64,
    pub already_absent: bool,
}

impl RuntimeRemoval {
    pub const SCHEMA: &'static str = "a3s.runtime.removal.v1";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime removal schema {:?}",
                self.schema
            ));
        }
        super::validate_id("request_id", &self.request_id, 512)?;
        super::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0 {
            return Err("removal generation must be positive".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLogQuery {
    pub unit_id: String,
    pub generation: u64,
    pub cursor: Option<String>,
    pub limit: u32,
    pub stream: Option<RuntimeLogStream>,
}

impl RuntimeLogQuery {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0 || self.limit == 0 || self.limit > 10_000 {
            return Err("log generation or limit is invalid".into());
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 1024 || value.contains('\0'))
        {
            return Err("log cursor is invalid".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLogChunk {
    pub cursor: String,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub stream: RuntimeLogStream,
    pub data: String,
}

impl RuntimeLogChunk {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_nonempty("log cursor", &self.cursor, 1024)?;
        if self.data.len() > 1024 * 1024 {
            return Err("log chunk exceeds one MiB".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExecRequest {
    pub request_id: String,
    pub unit_id: String,
    pub generation: u64,
    pub command: Vec<String>,
    pub timeout_ms: u64,
}

impl RuntimeExecRequest {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_id("request_id", &self.request_id, 512)?;
        super::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0
            || self.timeout_ms == 0
            || self.command.is_empty()
            || self.command.len() > 256
            || self
                .command
                .iter()
                .any(|value| value.is_empty() || value.len() > 32 * 1024 || value.contains('\0'))
        {
            return Err("exec request is invalid".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExecResult {
    pub request_id: String,
    pub observation: RuntimeObservation,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

impl RuntimeExecResult {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_id("request_id", &self.request_id, 512)?;
        self.observation.validate()?;
        if self.stdout.len() > 16 * 1024 * 1024 || self.stderr.len() > 16 * 1024 * 1024 {
            return Err("exec output exceeds protocol limits".into());
        }
        Ok(())
    }
}

fn canonical_digest<T: Serialize>(value: &T, valid: Result<(), String>) -> Result<String, String> {
    valid?;
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("could not encode Runtime request: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}
