use super::{verify_runtime_base, RuntimeBaseConformanceCase, RuntimeBaseConformanceReport};
use crate::contract::{RuntimeCapabilities, RuntimeFeature};
use crate::{RuntimeClient, RuntimeError, RuntimeResult};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};

/// Composable provider conformance profiles owned by the shared Runtime suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeConformanceProfile {
    Base,
    Recovery,
    Networking,
    Mounts,
    Health,
    Resources,
    Logs,
    Exec,
    Security,
    Outputs,
    Evidence,
}

impl RuntimeConformanceProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Recovery => "recovery",
            Self::Networking => "networking",
            Self::Mounts => "mounts",
            Self::Health => "health",
            Self::Resources => "resources",
            Self::Logs => "logs",
            Self::Exec => "exec",
            Self::Security => "security",
            Self::Outputs => "outputs",
            Self::Evidence => "evidence",
        }
    }
}

/// Exact shared case IDs and capability claims required for one profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConformanceProfileRequirements {
    pub profile: RuntimeConformanceProfile,
    pub case_ids: BTreeSet<String>,
    pub capability_claims: BTreeSet<String>,
}

/// Provider evidence returned for one non-Base profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConformanceProfileEvidence {
    pub profile: RuntimeConformanceProfile,
    pub case_ids: BTreeSet<String>,
    pub capability_claims: BTreeSet<String>,
}

/// Canonical pre/post provider inventory. Keys can represent provider
/// resources, volumes, ports, mounts, processes, or other retained objects;
/// values are provider-owned stable state digests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeConformanceInventory {
    pub entries: BTreeMap<String, String>,
}

/// Complete report for one profile-driven provider certification run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConformanceSuiteReport {
    pub base: RuntimeBaseConformanceReport,
    pub profiles: Vec<RuntimeConformanceProfileEvidence>,
    pub inventory_before: RuntimeConformanceInventory,
    pub inventory_after: RuntimeConformanceInventory,
}

/// Provider integration boundary for destructive, capability-specific
/// fixtures. No method has a success-by-default implementation: a production
/// provider must explicitly declare, execute, clean up, and inventory every
/// activated profile.
#[async_trait]
pub trait RuntimeConformanceFixture: Send + Sync {
    fn base_case(&self) -> &RuntimeBaseConformanceCase;

    fn available_profiles(&self) -> BTreeSet<RuntimeConformanceProfile>;

    async fn inventory(&self) -> RuntimeResult<RuntimeConformanceInventory>;

    async fn run_profile(
        &self,
        client: &dyn RuntimeClient,
        capabilities: &RuntimeCapabilities,
        profile: RuntimeConformanceProfile,
    ) -> RuntimeResult<RuntimeConformanceProfileEvidence>;

    async fn cleanup(&self) -> RuntimeResult<()>;
}

/// Derives mandatory and capability-triggered profiles from source-reported
/// capabilities. Base and Recovery are unconditional.
pub fn required_runtime_profiles(
    capabilities: &RuntimeCapabilities,
) -> RuntimeResult<BTreeSet<RuntimeConformanceProfile>> {
    capabilities.validate().map_err(RuntimeError::Protocol)?;
    let mut profiles = BTreeSet::from([
        RuntimeConformanceProfile::Base,
        RuntimeConformanceProfile::Recovery,
    ]);
    if !capabilities.network_modes.is_empty() {
        profiles.insert(RuntimeConformanceProfile::Networking);
    }
    if !capabilities.mount_kinds.is_empty() {
        profiles.insert(RuntimeConformanceProfile::Mounts);
    }
    if !capabilities.health_check_kinds.is_empty() {
        profiles.insert(RuntimeConformanceProfile::Health);
    }
    if !capabilities.resource_controls.is_empty() {
        profiles.insert(RuntimeConformanceProfile::Resources);
    }
    if capabilities.supports_feature(RuntimeFeature::Logs) {
        profiles.insert(RuntimeConformanceProfile::Logs);
    }
    if capabilities.supports_feature(RuntimeFeature::Exec) {
        profiles.insert(RuntimeConformanceProfile::Exec);
    }
    if !capabilities.isolation_levels.is_empty()
        || capabilities.supports_feature(RuntimeFeature::SecretReferences)
    {
        profiles.insert(RuntimeConformanceProfile::Security);
    }
    if capabilities.supports_feature(RuntimeFeature::OutputArtifacts) {
        profiles.insert(RuntimeConformanceProfile::Outputs);
    }
    if capabilities.supports_feature(RuntimeFeature::Usage)
        || capabilities.supports_feature(RuntimeFeature::Attestation)
    {
        profiles.insert(RuntimeConformanceProfile::Evidence);
    }
    Ok(profiles)
}

/// Returns the stable shared requirements a provider evidence record must
/// cover for the selected profile and capability set.
pub fn runtime_profile_requirements(
    capabilities: &RuntimeCapabilities,
    profile: RuntimeConformanceProfile,
) -> RuntimeResult<RuntimeConformanceProfileRequirements> {
    capabilities.validate().map_err(RuntimeError::Protocol)?;
    let mut case_ids = case_ids(profile)
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if profile == RuntimeConformanceProfile::Security
        && capabilities.supports_feature(RuntimeFeature::SecretReferences)
    {
        case_ids.insert("SECURITY-SECRET-NONDISCLOSURE".into());
    }
    if profile == RuntimeConformanceProfile::Evidence
        && capabilities.supports_feature(RuntimeFeature::Attestation)
    {
        case_ids.insert("EVIDENCE-ATTESTATION-VALIDITY".into());
    }
    let capability_claims = capability_claims(capabilities, profile);
    Ok(RuntimeConformanceProfileRequirements {
        profile,
        case_ids,
        capability_claims,
    })
}

/// Runs Base internally, activates every required/declared profile, validates
/// typed evidence, always requests provider cleanup, and rejects any inventory
/// delta.
pub async fn verify_runtime_profiles(
    client: &dyn RuntimeClient,
    fixture: &dyn RuntimeConformanceFixture,
) -> RuntimeResult<RuntimeConformanceSuiteReport> {
    let capabilities = client.capabilities().await?;
    capabilities.validate().map_err(RuntimeError::Protocol)?;
    let missing_lifecycle = [
        RuntimeFeature::DurableIdentity,
        RuntimeFeature::Stop,
        RuntimeFeature::Remove,
    ]
    .into_iter()
    .filter(|feature| !capabilities.supports_feature(*feature))
    .map(|feature| format!("feature:{feature:?}"))
    .collect::<Vec<_>>();
    if !missing_lifecycle.is_empty() {
        return Err(RuntimeError::UnsupportedCapabilities(missing_lifecycle));
    }
    fixture
        .base_case()
        .validate()
        .map_err(RuntimeError::InvalidRequest)?;
    for spec in fixture.base_case().specifications() {
        let missing = capabilities
            .missing_for(spec)
            .map_err(RuntimeError::InvalidRequest)?;
        if !missing.is_empty() {
            return Err(RuntimeError::UnsupportedCapabilities(missing));
        }
    }

    let required = required_runtime_profiles(&capabilities)?;
    let mut selected = fixture.available_profiles();
    selected.insert(RuntimeConformanceProfile::Base);
    let missing_profiles = required.difference(&selected).copied().collect::<Vec<_>>();
    if !missing_profiles.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "conformance fixture omits required profiles: {}",
            missing_profiles
                .iter()
                .map(|profile| profile.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    selected.extend(required);

    let inventory_before = fixture.inventory().await?;
    let execution: RuntimeResult<(
        RuntimeBaseConformanceReport,
        Vec<RuntimeConformanceProfileEvidence>,
    )> = async {
        let base = verify_runtime_base(client, fixture.base_case()).await?;
        let mut evidence = vec![base_evidence(&capabilities)?];
        for profile in selected
            .iter()
            .copied()
            .filter(|profile| *profile != RuntimeConformanceProfile::Base)
        {
            let actual = fixture.run_profile(client, &capabilities, profile).await?;
            validate_evidence(&capabilities, profile, &actual)?;
            evidence.push(actual);
        }
        Ok((base, evidence))
    }
    .await;

    let cleanup = fixture.cleanup().await;
    let inventory_after = fixture.inventory().await;
    cleanup?;
    let inventory_after = inventory_after?;
    if inventory_after != inventory_before {
        return Err(RuntimeError::Protocol(format!(
            "conformance cleanup changed provider inventory: before={:?}, after={:?}",
            inventory_before.entries, inventory_after.entries
        )));
    }
    let (base, profiles) = execution?;
    Ok(RuntimeConformanceSuiteReport {
        base,
        profiles,
        inventory_before,
        inventory_after,
    })
}

fn validate_evidence(
    capabilities: &RuntimeCapabilities,
    profile: RuntimeConformanceProfile,
    evidence: &RuntimeConformanceProfileEvidence,
) -> RuntimeResult<()> {
    if evidence.profile != profile {
        return Err(RuntimeError::Protocol(format!(
            "conformance {} fixture returned {} evidence",
            profile.as_str(),
            evidence.profile.as_str()
        )));
    }
    let required = runtime_profile_requirements(capabilities, profile)?;
    let missing_cases = required
        .case_ids
        .difference(&evidence.case_ids)
        .cloned()
        .collect::<Vec<_>>();
    let missing_claims = required
        .capability_claims
        .difference(&evidence.capability_claims)
        .cloned()
        .collect::<Vec<_>>();
    if !missing_cases.is_empty() || !missing_claims.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "conformance {} evidence is incomplete: missing cases {:?}, missing claims {:?}",
            profile.as_str(),
            missing_cases,
            missing_claims
        )));
    }
    Ok(())
}

fn base_evidence(
    capabilities: &RuntimeCapabilities,
) -> RuntimeResult<RuntimeConformanceProfileEvidence> {
    let required = runtime_profile_requirements(capabilities, RuntimeConformanceProfile::Base)?;
    Ok(RuntimeConformanceProfileEvidence {
        profile: required.profile,
        case_ids: required.case_ids,
        capability_claims: required.capability_claims,
    })
}

fn case_ids(profile: RuntimeConformanceProfile) -> &'static [&'static str] {
    match profile {
        RuntimeConformanceProfile::Base => &[
            "BASE-TASK-SUCCESS",
            "BASE-TASK-FAILURE",
            "BASE-TASK-TIMEOUT",
            "BASE-SERVICE-LIFECYCLE",
            "BASE-EXACT-REPLAY",
            "BASE-GENERATION-CONFLICT",
            "BASE-TOMBSTONE",
        ],
        RuntimeConformanceProfile::Recovery => &[
            "RECOVERY-CREATE-BEFORE-ACK",
            "RECOVERY-CLIENT-RESTART",
            "RECOVERY-PROVIDER-RESTART",
            "RECOVERY-EXTERNAL-DELETION",
            "RECOVERY-SAME-GENERATION-REPLACEMENT",
            "RECOVERY-DUPLICATE-DETECTION",
        ],
        RuntimeConformanceProfile::Networking => {
            &["NETWORK-MODE-BEHAVIOR", "NETWORK-PORT-COLLISION"]
        }
        RuntimeConformanceProfile::Mounts => &["MOUNT-BEHAVIOR", "MOUNT-ISOLATION-CLEANUP"],
        RuntimeConformanceProfile::Health => {
            &["HEALTH-PROBE-BEHAVIOR", "HEALTH-THRESHOLD-TRANSITION"]
        }
        RuntimeConformanceProfile::Resources => {
            &["RESOURCE-PROVIDER-INSPECT", "RESOURCE-WORKLOAD-BEHAVIOR"]
        }
        RuntimeConformanceProfile::Logs => &["LOG-ORDER-FILTER", "LOG-CURSOR-RETENTION"],
        RuntimeConformanceProfile::Exec => &["EXEC-STATE-BINDING", "EXEC-TIMEOUT-REPLAY"],
        RuntimeConformanceProfile::Security => {
            &["SECURITY-DIGEST-NAMESPACE", "SECURITY-METADATA-TAMPER"]
        }
        RuntimeConformanceProfile::Outputs => &["OUTPUT-EXACT-BOUNDED", "OUTPUT-DIGEST-BINDING"],
        RuntimeConformanceProfile::Evidence => &["EVIDENCE-SPEC-BINDING"],
    }
}

fn capability_claims(
    capabilities: &RuntimeCapabilities,
    profile: RuntimeConformanceProfile,
) -> BTreeSet<String> {
    match profile {
        RuntimeConformanceProfile::Base => capabilities
            .unit_classes
            .iter()
            .map(|class| format!("unit_class:{class:?}"))
            .collect(),
        RuntimeConformanceProfile::Recovery => BTreeSet::from(["feature:DurableIdentity".into()]),
        RuntimeConformanceProfile::Networking => capabilities
            .network_modes
            .iter()
            .map(|mode| format!("network_mode:{mode:?}"))
            .collect(),
        RuntimeConformanceProfile::Mounts => capabilities
            .mount_kinds
            .iter()
            .map(|kind| format!("mount_kind:{kind:?}"))
            .collect(),
        RuntimeConformanceProfile::Health => capabilities
            .health_check_kinds
            .iter()
            .map(|kind| format!("health_check:{kind:?}"))
            .collect(),
        RuntimeConformanceProfile::Resources => capabilities
            .resource_controls
            .iter()
            .map(|control| format!("resource_control:{control:?}"))
            .collect(),
        RuntimeConformanceProfile::Logs => BTreeSet::from(["feature:Logs".into()]),
        RuntimeConformanceProfile::Exec => BTreeSet::from(["feature:Exec".into()]),
        RuntimeConformanceProfile::Security => {
            let mut claims = capabilities
                .isolation_levels
                .iter()
                .map(|level| format!("isolation:{level:?}"))
                .collect::<BTreeSet<_>>();
            claims.insert("feature:DurableIdentity".into());
            if capabilities.supports_feature(RuntimeFeature::SecretReferences) {
                claims.insert("feature:SecretReferences".into());
            }
            claims
        }
        RuntimeConformanceProfile::Outputs => BTreeSet::from(["feature:OutputArtifacts".into()]),
        RuntimeConformanceProfile::Evidence => [RuntimeFeature::Usage, RuntimeFeature::Attestation]
            .into_iter()
            .filter(|feature| capabilities.supports_feature(*feature))
            .map(|feature| format!("feature:{feature:?}"))
            .collect(),
    }
}
