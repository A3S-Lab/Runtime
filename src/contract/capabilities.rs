use super::{
    HealthCheckKind, IsolationLevel, MountKind, NetworkMode, RuntimeUnitClass, RuntimeUnitSpec,
};
use crate::ProviderId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceControl {
    Cpu,
    Memory,
    Pids,
    EphemeralStorage,
    ExecutionTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFeature {
    DurableIdentity,
    Stop,
    Remove,
    Logs,
    Exec,
    Usage,
    Attestation,
    SecretReferences,
    OutputArtifacts,
}

/// Structured, provider-reported capabilities. Product-specific support
/// predicates belong to the caller, not this protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapabilities {
    pub schema: String,
    pub provider_id: ProviderId,
    pub provider_build: String,
    pub unit_classes: Vec<RuntimeUnitClass>,
    pub artifact_media_types: Vec<String>,
    pub isolation_levels: Vec<IsolationLevel>,
    pub network_modes: Vec<NetworkMode>,
    pub mount_kinds: Vec<MountKind>,
    pub health_check_kinds: Vec<HealthCheckKind>,
    pub resource_controls: Vec<ResourceControl>,
    pub features: Vec<RuntimeFeature>,
}

impl RuntimeCapabilities {
    pub const SCHEMA: &'static str = "a3s.runtime.capabilities.v3";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime capabilities schema {:?}",
                self.schema
            ));
        }
        super::validate_nonempty("provider_build", &self.provider_build, 255)?;
        if self.unit_classes.is_empty()
            || self.artifact_media_types.is_empty()
            || self.isolation_levels.is_empty()
            || self.resource_controls.is_empty()
        {
            return Err("Runtime capabilities omit a required capability family".into());
        }
        ensure_unique("unit class", &self.unit_classes)?;
        ensure_unique("artifact media type", &self.artifact_media_types)?;
        ensure_unique("isolation level", &self.isolation_levels)?;
        ensure_unique("network mode", &self.network_modes)?;
        ensure_unique("mount kind", &self.mount_kinds)?;
        ensure_unique("health check kind", &self.health_check_kinds)?;
        ensure_unique("resource control", &self.resource_controls)?;
        ensure_unique("feature", &self.features)?;
        for media_type in &self.artifact_media_types {
            super::validate_nonempty("artifact media type", media_type, 255)?;
        }
        Ok(())
    }

    pub fn supports_feature(&self, feature: RuntimeFeature) -> bool {
        self.features.contains(&feature)
    }

    pub fn missing_for(&self, spec: &RuntimeUnitSpec) -> Result<Vec<String>, String> {
        self.validate()?;
        spec.validate()?;
        let mut missing = Vec::new();
        if !self.unit_classes.contains(&spec.class) {
            missing.push(format!("unit_class:{:?}", spec.class));
        }
        if !self
            .artifact_media_types
            .contains(&spec.artifact.media_type)
        {
            missing.push(format!("artifact_media_type:{}", spec.artifact.media_type));
        }
        if !self.isolation_levels.contains(&spec.isolation) {
            missing.push(format!("isolation:{:?}", spec.isolation));
        }
        if !self.network_modes.contains(&spec.network.mode) {
            missing.push(format!("network_mode:{:?}", spec.network.mode));
        }
        for kind in spec.mounts.iter().map(|mount| mount.source.kind()) {
            if !self.mount_kinds.contains(&kind) {
                missing.push(format!("mount_kind:{kind:?}"));
            }
        }
        if let Some(health) = &spec.health {
            let kind = health.probe.kind();
            if !self.health_check_kinds.contains(&kind) {
                missing.push(format!("health_check:{kind:?}"));
            }
        }
        for required in [
            ResourceControl::Cpu,
            ResourceControl::Memory,
            ResourceControl::Pids,
        ] {
            if !self.resource_controls.contains(&required) {
                missing.push(format!("resource_control:{required:?}"));
            }
        }
        if spec.resources.ephemeral_storage_bytes.is_some()
            && !self
                .resource_controls
                .contains(&ResourceControl::EphemeralStorage)
        {
            missing.push("resource_control:EphemeralStorage".into());
        }
        if spec.resources.execution_timeout_ms.is_some()
            && !self
                .resource_controls
                .contains(&ResourceControl::ExecutionTimeout)
        {
            missing.push("resource_control:ExecutionTimeout".into());
        }
        if !self.supports_feature(RuntimeFeature::DurableIdentity) {
            missing.push("feature:DurableIdentity".into());
        }
        if !spec.secrets.is_empty() && !self.supports_feature(RuntimeFeature::SecretReferences) {
            missing.push("feature:SecretReferences".into());
        }
        if !spec.outputs.is_empty() && !self.supports_feature(RuntimeFeature::OutputArtifacts) {
            missing.push("feature:OutputArtifacts".into());
        }
        if spec.isolation == IsolationLevel::Confidential
            && !self.supports_feature(RuntimeFeature::Attestation)
        {
            missing.push("feature:Attestation".into());
        }
        missing.sort();
        missing.dedup();
        Ok(missing)
    }
}

fn ensure_unique<T>(label: &str, values: &[T]) -> Result<(), String>
where
    T: Ord + Clone,
{
    let unique = values.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != values.len() {
        return Err(format!(
            "Runtime capabilities contain duplicate {label} values"
        ));
    }
    Ok(())
}
