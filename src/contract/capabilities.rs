use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapabilities {
    pub schema: String,
    pub semantics_profile_digest: String,
    pub provider_build: String,
    pub immutable_assets: bool,
    pub role_isolation: bool,
    pub protected_mounts: bool,
    pub protected_typed_results: bool,
    pub terminal_checkpoints: bool,
    pub submission_projection: bool,
    pub network_none: bool,
    pub hard_resource_limits: bool,
    pub durable_operations: bool,
    pub cancellation: bool,
    pub usage_evidence: bool,
}

impl RuntimeCapabilities {
    pub const SCHEMA: &'static str = "a3s.runtime.capabilities.v1";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime capabilities schema {:?}",
                self.schema
            ));
        }
        super::artifact::validate_digest(&self.semantics_profile_digest)?;
        if self.provider_build.trim().is_empty() {
            return Err("provider_build must not be empty".into());
        }
        Ok(())
    }

    pub fn supports_bench_p1(&self) -> bool {
        self.immutable_assets
            && self.role_isolation
            && self.protected_mounts
            && self.protected_typed_results
            && self.terminal_checkpoints
            && self.submission_projection
            && self.network_none
            && self.hard_resource_limits
            && self.durable_operations
            && self.cancellation
            && self.usage_evidence
    }
}
