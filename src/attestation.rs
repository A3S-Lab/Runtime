use crate::contract::{
    ArtifactRef, IsolationLevel, RuntimeEvidence, RuntimeObservation, RuntimeUnitClass,
    RuntimeUnitSpec, RuntimeUnitState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Exact provider-neutral proof that one provider resource observed one
/// identity-attached Runtime Unit generation.
///
/// Product policy remains outside Runtime. The attachment digest is opaque;
/// this projection only proves that the same immutable digest entered the
/// specification, provider evidence, and attested provider observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAttestationBinding {
    pub unit_id: String,
    pub generation: u64,
    pub class: RuntimeUnitClass,
    pub isolation: IsolationLevel,
    pub state: RuntimeUnitState,
    pub spec_digest: String,
    pub identity_attachment_digest: String,
    pub provider_resource_id: String,
    pub provider_build: String,
    pub observed_at_ms: u64,
    pub evidence: RuntimeEvidence,
    pub provider_attestation: ArtifactRef,
}

impl RuntimeAttestationBinding {
    /// Project a closed binding from an exact specification and observation.
    /// Missing, stale-generation, drifted, or unattested evidence fails
    /// closed. Freshness and product policy admission remain caller-owned.
    pub fn from_observation(
        spec: &RuntimeUnitSpec,
        observation: &RuntimeObservation,
    ) -> Result<Self, String> {
        observation.validate_against(spec)?;
        let identity_attachment_digest = spec
            .identity_attachment_digest
            .as_ref()
            .ok_or_else(|| "Runtime specification has no identity attachment".to_string())?;
        let provider_resource_id = observation
            .provider_resource_id
            .as_ref()
            .ok_or_else(|| "Runtime attestation has no provider resource identity".to_string())?;
        let provider_build = observation
            .provider_build
            .as_ref()
            .ok_or_else(|| "Runtime attestation has no provider build identity".to_string())?;
        let evidence = observation
            .evidence
            .as_ref()
            .ok_or_else(|| "Runtime attestation has no provider evidence".to_string())?;
        if evidence.provider_build != *provider_build {
            return Err("Runtime attestation provider build evidence drifted".into());
        }
        if evidence.identity_attachment_digest.as_ref() != Some(identity_attachment_digest) {
            return Err("Runtime attestation identity attachment evidence drifted".into());
        }
        let provider_attestation = observation
            .provider_attestation
            .as_ref()
            .ok_or_else(|| "Runtime observation has no provider attestation".to_string())?;
        if observation.observed_at_ms == 0 {
            return Err("Runtime attestation observation time must be positive".into());
        }
        let value = Self {
            unit_id: spec.unit_id.clone(),
            generation: spec.generation,
            class: spec.class,
            isolation: spec.isolation,
            state: observation.state,
            spec_digest: observation.spec_digest.clone(),
            identity_attachment_digest: identity_attachment_digest.clone(),
            provider_resource_id: provider_resource_id.clone(),
            provider_build: provider_build.clone(),
            observed_at_ms: observation.observed_at_ms,
            evidence: evidence.clone(),
            provider_attestation: provider_attestation.clone(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), String> {
        crate::contract::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0 || self.observed_at_ms == 0 {
            return Err(
                "Runtime attestation generation and observation time must be positive".into(),
            );
        }
        crate::contract::validate_digest(&self.spec_digest)?;
        crate::contract::validate_digest(&self.identity_attachment_digest)?;
        crate::contract::validate_nonempty(
            "provider_resource_id",
            &self.provider_resource_id,
            1024,
        )?;
        crate::contract::validate_nonempty("provider_build", &self.provider_build, 255)?;
        self.evidence.validate()?;
        self.provider_attestation.validate()?;
        if self.evidence.provider_build != self.provider_build
            || self.evidence.spec_digest != self.spec_digest
            || self.evidence.identity_attachment_digest.as_ref()
                != Some(&self.identity_attachment_digest)
        {
            return Err("Runtime attestation evidence does not match its binding".into());
        }
        Ok(())
    }

    /// Stable digest for a caller-owned durable admission record.
    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        #[derive(Serialize)]
        struct CanonicalBinding<'a> {
            unit_id: &'a str,
            generation: u64,
            class: RuntimeUnitClass,
            isolation: IsolationLevel,
            state: RuntimeUnitState,
            spec_digest: &'a str,
            identity_attachment_digest: &'a str,
            provider_resource_id: &'a str,
            provider_build: &'a str,
            observed_at_ms: u64,
            evidence: &'a RuntimeEvidence,
            provider_attestation: &'a ArtifactRef,
        }
        let bytes = serde_json::to_vec(&CanonicalBinding {
            unit_id: &self.unit_id,
            generation: self.generation,
            class: self.class,
            isolation: self.isolation,
            state: self.state,
            spec_digest: &self.spec_digest,
            identity_attachment_digest: &self.identity_attachment_digest,
            provider_resource_id: &self.provider_resource_id,
            provider_build: &self.provider_build,
            observed_at_ms: self.observed_at_ms,
            evidence: &self.evidence,
            provider_attestation: &self.provider_attestation,
        })
        .map_err(|error| format!("could not encode Runtime attestation binding: {error}"))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}
