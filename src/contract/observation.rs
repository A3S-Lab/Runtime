use super::{ArtifactRef, RuntimeOutputArtifact, RuntimeUnitClass, RuntimeUnitSpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUnitState {
    Accepted,
    Preparing,
    Starting,
    Running,
    Stopping,
    Stopped,
    Succeeded,
    Failed,
    Unknown,
}

impl RuntimeUnitState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeHealthState {
    Unknown,
    Starting,
    Healthy,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHealthObservation {
    pub state: RuntimeHealthState,
    pub checked_at_ms: u64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeUsage {
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub peak_memory_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub storage_read_bytes: u64,
    pub storage_write_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl RuntimeFailure {
    fn validate(&self) -> Result<(), String> {
        super::validate_name("failure code", &self.code)?;
        super::validate_nonempty("failure message", &self.message, 16 * 1024)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEvidence {
    pub provider_build: String,
    pub spec_digest: String,
    pub semantics_profile_digest: Option<String>,
    pub claims: BTreeMap<String, String>,
}

impl RuntimeEvidence {
    fn validate(&self) -> Result<(), String> {
        super::validate_nonempty("provider_build", &self.provider_build, 255)?;
        super::validate_digest(&self.spec_digest)?;
        if let Some(digest) = &self.semantics_profile_digest {
            super::validate_digest(digest)?;
        }
        if self.claims.len() > 128
            || self
                .claims
                .iter()
                .any(|(key, value)| key.len() > 255 || value.len() > 4096)
        {
            return Err("Runtime evidence claims exceed protocol limits".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeObservation {
    pub schema: String,
    pub unit_id: String,
    pub generation: u64,
    pub spec_digest: String,
    pub class: RuntimeUnitClass,
    pub state: RuntimeUnitState,
    pub provider_resource_id: Option<String>,
    pub provider_build: Option<String>,
    pub observed_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub health: Option<RuntimeHealthObservation>,
    pub outputs: Vec<RuntimeOutputArtifact>,
    pub usage: Option<RuntimeUsage>,
    pub evidence: Option<RuntimeEvidence>,
    pub provider_attestation: Option<ArtifactRef>,
    pub failure: Option<RuntimeFailure>,
}

impl RuntimeObservation {
    pub const SCHEMA: &'static str = "a3s.runtime.observation.v1";

    pub(crate) fn accepted(spec: &RuntimeUnitSpec, observed_at_ms: u64) -> Result<Self, String> {
        Ok(Self {
            schema: Self::SCHEMA.into(),
            unit_id: spec.unit_id.clone(),
            generation: spec.generation,
            spec_digest: spec.digest()?,
            class: spec.class,
            state: RuntimeUnitState::Accepted,
            provider_resource_id: None,
            provider_build: None,
            observed_at_ms,
            started_at_ms: None,
            finished_at_ms: None,
            health: None,
            outputs: Vec::new(),
            usage: None,
            evidence: None,
            provider_attestation: None,
            failure: None,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime observation schema {:?}",
                self.schema
            ));
        }
        super::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0 {
            return Err("Runtime observation generation must be positive".into());
        }
        super::validate_digest(&self.spec_digest)?;
        if let Some(value) = &self.provider_resource_id {
            super::validate_nonempty("provider_resource_id", value, 1024)?;
        }
        if let Some(value) = &self.provider_build {
            super::validate_nonempty("provider_build", value, 255)?;
        }
        if !matches!(
            self.state,
            RuntimeUnitState::Accepted | RuntimeUnitState::Unknown
        ) && (self.provider_resource_id.is_none() || self.provider_build.is_none())
        {
            return Err("provider-backed observations require resource and build identity".into());
        }
        if let (Some(started), Some(finished)) = (self.started_at_ms, self.finished_at_ms) {
            if finished < started {
                return Err("finished_at_ms precedes started_at_ms".into());
            }
        }
        if self.state.is_terminal() != self.finished_at_ms.is_some() {
            return Err("terminal state and finished_at_ms do not agree".into());
        }
        if self.state == RuntimeUnitState::Failed {
            self.failure
                .as_ref()
                .ok_or_else(|| "failed observation is missing failure".to_string())?
                .validate()?;
        } else if self.failure.is_some() {
            return Err("non-failed observation contains failure".into());
        }
        if self.class == RuntimeUnitClass::Service && self.state == RuntimeUnitState::Succeeded {
            return Err("Service cannot enter succeeded state".into());
        }
        if self.class == RuntimeUnitClass::Task && self.health.is_some() {
            return Err("Task observation cannot contain Service health".into());
        }
        if !(self.outputs.is_empty()
            || self.class == RuntimeUnitClass::Task && self.state == RuntimeUnitState::Succeeded)
        {
            return Err("output artifacts require a succeeded Task".into());
        }
        let mut output_names = BTreeSet::new();
        for output in &self.outputs {
            output.validate()?;
            if !output_names.insert(&output.name) {
                return Err(format!("duplicate output artifact {:?}", output.name));
            }
        }
        if let Some(health) = &self.health {
            if let Some(message) = &health.message {
                super::validate_nonempty("health message", message, 4096)?;
            }
        }
        if let Some(evidence) = &self.evidence {
            evidence.validate()?;
            if evidence.spec_digest != self.spec_digest {
                return Err("Runtime evidence does not bind the observation spec".into());
            }
        }
        if let Some(attestation) = &self.provider_attestation {
            attestation.validate()?;
        }
        Ok(())
    }

    pub fn validate_against(&self, spec: &RuntimeUnitSpec) -> Result<(), String> {
        self.validate()?;
        spec.validate()?;
        if self.unit_id != spec.unit_id
            || self.generation != spec.generation
            || self.class != spec.class
            || self.spec_digest != spec.digest()?
        {
            return Err("Runtime observation does not match the unit specification".into());
        }
        Ok(())
    }

    pub fn converges(&self, spec: &RuntimeUnitSpec) -> bool {
        if self.validate_against(spec).is_err() {
            return false;
        }
        match spec.class {
            RuntimeUnitClass::Task => self.state == RuntimeUnitState::Succeeded,
            RuntimeUnitClass::Service => {
                self.state == RuntimeUnitState::Running
                    && spec.health.as_ref().is_none_or(|_| {
                        self.health
                            .as_ref()
                            .is_some_and(|health| health.state == RuntimeHealthState::Healthy)
                    })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeInspection {
    Found {
        observation: Box<RuntimeObservation>,
    },
    NotFound {
        unit_id: String,
        last_generation: Option<u64>,
    },
}

impl RuntimeInspection {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Found { observation } => observation.validate(),
            Self::NotFound {
                unit_id,
                last_generation,
            } => {
                super::validate_id("unit_id", unit_id, 512)?;
                if *last_generation == Some(0) {
                    return Err("last_generation must be positive when present".into());
                }
                Ok(())
            }
        }
    }
}
