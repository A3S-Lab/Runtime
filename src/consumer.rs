use crate::contract::{
    NetworkMode, RuntimeCapabilities, RuntimeFeature, RuntimeHealthState, RuntimeObservation,
    RuntimeUnitClass, RuntimeUnitSpec, RuntimeUnitState,
};
use crate::{RuntimeAttestationBinding, RuntimeError, RuntimeResult};
use std::collections::BTreeSet;

/// Provider-neutral requirements imposed by one Runtime consumer profile.
///
/// This is an admission and readiness abstraction, not a wire-contract type.
/// Callers retain ownership of their domain semantics and compose them from the
/// two generic Runtime unit classes and advertised capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConsumerRequirements {
    unit_class: RuntimeUnitClass,
    required_features: BTreeSet<RuntimeFeature>,
    semantics_profile_required: bool,
    health_required: bool,
    service_endpoints_required: bool,
    identity_attestation_required: bool,
}

impl RuntimeConsumerRequirements {
    pub fn new(unit_class: RuntimeUnitClass) -> Self {
        Self {
            unit_class,
            required_features: BTreeSet::new(),
            semantics_profile_required: false,
            health_required: false,
            service_endpoints_required: false,
            identity_attestation_required: false,
        }
    }

    pub fn require_feature(mut self, feature: RuntimeFeature) -> Self {
        self.required_features.insert(feature);
        self
    }

    /// Requires the immutable consumer-owned semantics profile to be present
    /// in the specification and repeated by provider evidence.
    pub fn require_semantics_profile(mut self) -> Self {
        self.semantics_profile_required = true;
        self
    }

    /// Requires a Service health policy and a healthy running observation.
    pub fn require_health(mut self) -> Self {
        self.health_required = true;
        self
    }

    /// Requires a Service network declaration and exact observed endpoints.
    pub fn require_service_endpoints(mut self) -> Self {
        self.service_endpoints_required = true;
        self
    }

    /// Requires one opaque identity attachment in the desired specification
    /// and exact generation-bound provider attestation in the observation.
    pub fn require_identity_attestation(mut self) -> Self {
        self.identity_attestation_required = true;
        self.required_features
            .insert(RuntimeFeature::IdentityAttachment);
        self.required_features.insert(RuntimeFeature::Attestation);
        self
    }

    /// Fails closed unless the immutable unit specification can be fulfilled
    /// by the selected provider and satisfies this consumer profile.
    pub fn admit_spec(
        &self,
        spec: &RuntimeUnitSpec,
        capabilities: &RuntimeCapabilities,
    ) -> RuntimeResult<()> {
        self.validate().map_err(RuntimeError::InvalidRequest)?;
        spec.validate().map_err(RuntimeError::InvalidRequest)?;
        capabilities.validate().map_err(RuntimeError::Protocol)?;
        self.validate_spec_shape(spec)
            .map_err(RuntimeError::InvalidRequest)?;

        let mut missing = capabilities
            .missing_for(spec)
            .map_err(RuntimeError::Protocol)?;
        missing.extend(
            self.required_features
                .iter()
                .filter(|feature| !capabilities.supports_feature(**feature))
                .map(|feature| format!("feature:{feature:?}")),
        );
        missing.sort();
        missing.dedup();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::UnsupportedCapabilities(missing))
        }
    }

    /// Accepts an observation only when it is bound to the admitted unit and
    /// contains every readiness proof required by this consumer profile.
    pub fn accept_observation(
        &self,
        spec: &RuntimeUnitSpec,
        observation: &RuntimeObservation,
    ) -> RuntimeResult<()> {
        self.validate().map_err(RuntimeError::InvalidRequest)?;
        spec.validate().map_err(RuntimeError::InvalidRequest)?;
        self.validate_spec_shape(spec)
            .map_err(RuntimeError::InvalidRequest)?;
        observation
            .validate_against(spec)
            .map_err(RuntimeError::Protocol)?;

        if self.semantics_profile_required {
            let expected = spec
                .semantics_profile_digest
                .as_ref()
                .expect("validated consumer semantics profile");
            let actual = observation
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.semantics_profile_digest.as_ref());
            if actual != Some(expected) {
                return Err(RuntimeError::Protocol(
                    "Runtime observation omits exact semantics profile evidence".into(),
                ));
            }
        }

        if self.health_required
            && (observation.state != RuntimeUnitState::Running
                || observation.health.as_ref().map(|health| health.state)
                    != Some(RuntimeHealthState::Healthy))
        {
            return Err(RuntimeError::Protocol(
                "Runtime consumer requires a healthy running Service observation".into(),
            ));
        }

        if self.service_endpoints_required
            && observation
                .service_endpoints()
                .map_err(RuntimeError::Protocol)?
                .is_empty()
        {
            return Err(RuntimeError::Protocol(
                "Runtime consumer requires exact Service endpoint evidence".into(),
            ));
        }
        if self.identity_attestation_required {
            RuntimeAttestationBinding::from_observation(spec, observation)
                .map_err(RuntimeError::Protocol)?;
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if (self.health_required || self.service_endpoints_required)
            && self.unit_class != RuntimeUnitClass::Service
        {
            return Err("health and endpoint requirements apply only to Runtime Service".into());
        }
        Ok(())
    }

    fn validate_spec_shape(&self, spec: &RuntimeUnitSpec) -> Result<(), String> {
        if spec.class != self.unit_class {
            return Err(format!(
                "Runtime consumer requires {:?}, but the specification is {:?}",
                self.unit_class, spec.class
            ));
        }
        if self.semantics_profile_required && spec.semantics_profile_digest.is_none() {
            return Err("Runtime consumer requires a semantics profile digest".into());
        }
        if self.identity_attestation_required && spec.identity_attachment_digest.is_none() {
            return Err("Runtime consumer requires an identity attachment digest".into());
        }
        if self.health_required && spec.health.is_none() {
            return Err("Runtime consumer requires a Service health policy".into());
        }
        if self.service_endpoints_required
            && (spec.network.mode != NetworkMode::Service || spec.network.ports.is_empty())
        {
            return Err("Runtime consumer requires declared Service endpoints".into());
        }
        Ok(())
    }
}
