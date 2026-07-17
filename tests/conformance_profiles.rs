#[path = "process_races/driver.rs"]
mod driver;

use a3s_runtime::contract::{
    ArtifactRef, HealthCheckKind, IsolationLevel, MountKind, NetworkMode, ResourceLimits,
    RestartPolicy, RuntimeActionRequest, RuntimeApplyRequest, RuntimeFeature, RuntimeNetworkSpec,
    RuntimeProcessSpec, RuntimeUnitClass, RuntimeUnitSpec,
};
use a3s_runtime::{
    required_runtime_profiles, runtime_profile_requirements, verify_runtime_profiles,
    FileRuntimeStateStore, ManagedRuntimeClient, RuntimeConformanceCase, RuntimeConformanceFixture,
    RuntimeConformanceInventory, RuntimeConformanceProfile, RuntimeConformanceProfileEvidence,
    RuntimeDriver, RuntimeError, RuntimeResult,
};
use async_trait::async_trait;
use driver::{ProcessRaceDriver, IMAGE_MEDIA_TYPE};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn spec(unit_id: &str, class: RuntimeUnitClass, args: &[&str]) -> RuntimeUnitSpec {
    RuntimeUnitSpec {
        schema: RuntimeUnitSpec::SCHEMA.into(),
        unit_id: unit_id.into(),
        generation: 1,
        class,
        artifact: ArtifactRef {
            uri: format!(
                "oci://registry.example/a3s/conformance@sha256:{}",
                "a".repeat(64)
            ),
            digest: format!("sha256:{}", "a".repeat(64)),
            media_type: IMAGE_MEDIA_TYPE.into(),
        },
        process: RuntimeProcessSpec {
            command: vec!["/bin/fixture".into()],
            args: args.iter().copied().map(Into::into).collect(),
            working_directory: None,
            environment: BTreeMap::new(),
        },
        mounts: Vec::new(),
        secrets: Vec::new(),
        network: RuntimeNetworkSpec {
            mode: NetworkMode::None,
            ports: Vec::new(),
        },
        resources: ResourceLimits {
            cpu_millis: 100,
            memory_bytes: 64 * 1024 * 1024,
            pids: 32,
            ephemeral_storage_bytes: None,
            execution_timeout_ms: (class == RuntimeUnitClass::Task).then_some(1_000),
        },
        isolation: IsolationLevel::Container,
        health: None,
        restart: if class == RuntimeUnitClass::Task {
            RestartPolicy::Never
        } else {
            RestartPolicy::Always
        },
        outputs: Vec::new(),
        semantics_profile_digest: None,
    }
}

fn apply(request_id: &str, spec: RuntimeUnitSpec) -> RuntimeApplyRequest {
    RuntimeApplyRequest {
        schema: RuntimeApplyRequest::SCHEMA.into(),
        request_id: request_id.into(),
        deadline_at_ms: None,
        spec,
    }
}

fn action(request_id: &str, apply: &RuntimeApplyRequest) -> RuntimeActionRequest {
    RuntimeActionRequest {
        schema: RuntimeActionRequest::SCHEMA.into(),
        request_id: request_id.into(),
        unit_id: apply.spec.unit_id.clone(),
        generation: apply.spec.generation,
        deadline_at_ms: None,
    }
}

fn base_case() -> a3s_runtime::RuntimeBaseConformanceCase {
    let task_apply = apply(
        "base-task-apply",
        spec("base-task", RuntimeUnitClass::Task, &[]),
    );
    let service_apply = apply(
        "base-service-apply",
        spec("base-service", RuntimeUnitClass::Service, &[]),
    );
    let task_failure_apply = apply(
        "base-failure-apply",
        spec("base-failure", RuntimeUnitClass::Task, &["fail"]),
    );
    let task_timeout_apply = apply(
        "base-timeout-apply",
        spec("base-timeout", RuntimeUnitClass::Task, &["timeout"]),
    );
    let generation_apply = apply(
        "base-generation-apply",
        spec("base-generation", RuntimeUnitClass::Service, &[]),
    );
    let generation_conflict_apply = apply(
        "base-generation-conflict",
        spec("base-generation", RuntimeUnitClass::Service, &["changed"]),
    );
    a3s_runtime::RuntimeBaseConformanceCase {
        lifecycle: RuntimeConformanceCase {
            task_remove: action("base-task-remove", &task_apply),
            service_stop: action("base-service-stop", &service_apply),
            service_remove: action("base-service-remove", &service_apply),
            task_apply,
            service_apply,
        },
        task_failure_remove: action("base-failure-remove", &task_failure_apply),
        task_failure_apply,
        task_timeout_remove: action("base-timeout-remove", &task_timeout_apply),
        task_timeout_apply,
        generation_remove: action("base-generation-remove", &generation_apply),
        generation_apply,
        generation_conflict_apply,
    }
}

struct ProfileFixture {
    base: a3s_runtime::RuntimeBaseConformanceCase,
    driver: ProcessRaceDriver,
    available: BTreeSet<RuntimeConformanceProfile>,
    incomplete: Option<RuntimeConformanceProfile>,
    cleanup_calls: AtomicUsize,
}

impl ProfileFixture {
    fn new(provider_root: &Path) -> Self {
        Self {
            base: base_case(),
            driver: ProcessRaceDriver::new(provider_root),
            available: BTreeSet::from([
                RuntimeConformanceProfile::Recovery,
                RuntimeConformanceProfile::Networking,
                RuntimeConformanceProfile::Resources,
                RuntimeConformanceProfile::Security,
            ]),
            incomplete: None,
            cleanup_calls: AtomicUsize::new(0),
        }
    }

    fn unit_ids(&self) -> Vec<String> {
        vec![
            self.base.lifecycle.task_apply.spec.unit_id.clone(),
            self.base.lifecycle.service_apply.spec.unit_id.clone(),
            self.base.task_failure_apply.spec.unit_id.clone(),
            self.base.task_timeout_apply.spec.unit_id.clone(),
            self.base.generation_apply.spec.unit_id.clone(),
        ]
    }
}

#[async_trait]
impl RuntimeConformanceFixture for ProfileFixture {
    fn base_case(&self) -> &a3s_runtime::RuntimeBaseConformanceCase {
        &self.base
    }

    fn available_profiles(&self) -> BTreeSet<RuntimeConformanceProfile> {
        self.available.clone()
    }

    async fn inventory(&self) -> RuntimeResult<RuntimeConformanceInventory> {
        let driver = self.driver.clone();
        let unit_ids = self.unit_ids();
        tokio::task::spawn_blocking(move || {
            let mut entries = BTreeMap::new();
            for unit_id in unit_ids {
                for resource in driver.inventory(&unit_id)? {
                    entries.insert(
                        resource.resource_id,
                        format!("{}:{:?}", resource.generation, resource.state),
                    );
                }
            }
            Ok(RuntimeConformanceInventory { entries })
        })
        .await
        .map_err(|error| RuntimeError::Transport(format!("inventory task failed: {error}")))?
    }

    async fn run_profile(
        &self,
        _client: &dyn a3s_runtime::RuntimeClient,
        capabilities: &a3s_runtime::contract::RuntimeCapabilities,
        profile: RuntimeConformanceProfile,
    ) -> RuntimeResult<RuntimeConformanceProfileEvidence> {
        let requirements = runtime_profile_requirements(capabilities, profile)?;
        let mut evidence = RuntimeConformanceProfileEvidence {
            profile,
            case_ids: requirements.case_ids,
            capability_claims: requirements.capability_claims,
        };
        if self.incomplete == Some(profile) {
            let _ = evidence.case_ids.pop_first();
        }
        Ok(evidence)
    }

    async fn cleanup(&self) -> RuntimeResult<()> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn client(state_root: &Path, provider_root: &Path) -> ManagedRuntimeClient {
    ManagedRuntimeClient::new(
        Arc::new(FileRuntimeStateStore::new(state_root)),
        Arc::new(ProcessRaceDriver::new(provider_root)),
    )
}

#[tokio::test]
async fn conf_profile_001_base_and_capability_profiles_run_without_inventory_delta() {
    let state = tempfile::tempdir().expect("profile state root");
    let provider = tempfile::tempdir().expect("profile provider root");
    let fixture = ProfileFixture::new(provider.path());
    let report = verify_runtime_profiles(&client(state.path(), provider.path()), &fixture)
        .await
        .expect("verify profile suite");
    assert_eq!(fixture.cleanup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.inventory_before, report.inventory_after);
    assert_eq!(
        report
            .profiles
            .iter()
            .map(|evidence| evidence.profile)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            RuntimeConformanceProfile::Base,
            RuntimeConformanceProfile::Recovery,
            RuntimeConformanceProfile::Networking,
            RuntimeConformanceProfile::Resources,
            RuntimeConformanceProfile::Security,
        ])
    );
}

#[tokio::test]
async fn conf_profile_002_missing_mandatory_profile_fails_before_provider_work() {
    let state = tempfile::tempdir().expect("missing-profile state root");
    let provider = tempfile::tempdir().expect("missing-profile provider root");
    let mut fixture = ProfileFixture::new(provider.path());
    fixture
        .available
        .remove(&RuntimeConformanceProfile::Recovery);
    assert!(matches!(
        verify_runtime_profiles(&client(state.path(), provider.path()), &fixture).await,
        Err(RuntimeError::Protocol(message)) if message.contains("recovery")
    ));
    assert_eq!(fixture.cleanup_calls.load(Ordering::SeqCst), 0);
    assert!(fixture
        .driver
        .inventory("base-task")
        .expect("missing-profile inventory")
        .is_empty());
}

#[tokio::test]
async fn conf_profile_003_incomplete_evidence_fails_and_still_cleans_up() {
    let state = tempfile::tempdir().expect("incomplete-profile state root");
    let provider = tempfile::tempdir().expect("incomplete-profile provider root");
    let mut fixture = ProfileFixture::new(provider.path());
    fixture.incomplete = Some(RuntimeConformanceProfile::Recovery);
    assert!(matches!(
        verify_runtime_profiles(&client(state.path(), provider.path()), &fixture).await,
        Err(RuntimeError::Protocol(message)) if message.contains("evidence is incomplete")
    ));
    assert_eq!(fixture.cleanup_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn conf_profile_004_all_advertised_optional_families_activate() {
    let provider = tempfile::tempdir().expect("activation provider root");
    let driver = ProcessRaceDriver::new(provider.path());
    let mut capabilities = driver.capabilities().await.expect("driver capabilities");
    capabilities.mount_kinds = vec![MountKind::Volume];
    capabilities.health_check_kinds = vec![HealthCheckKind::Command];
    capabilities
        .isolation_levels
        .push(IsolationLevel::Confidential);
    capabilities.features.extend([
        RuntimeFeature::Logs,
        RuntimeFeature::Exec,
        RuntimeFeature::SecretReferences,
        RuntimeFeature::OutputArtifacts,
        RuntimeFeature::Usage,
        RuntimeFeature::Attestation,
    ]);
    let profiles = required_runtime_profiles(&capabilities).expect("derive profiles");
    assert_eq!(
        profiles,
        BTreeSet::from([
            RuntimeConformanceProfile::Base,
            RuntimeConformanceProfile::Recovery,
            RuntimeConformanceProfile::Networking,
            RuntimeConformanceProfile::Mounts,
            RuntimeConformanceProfile::Health,
            RuntimeConformanceProfile::Resources,
            RuntimeConformanceProfile::Logs,
            RuntimeConformanceProfile::Exec,
            RuntimeConformanceProfile::Security,
            RuntimeConformanceProfile::Outputs,
            RuntimeConformanceProfile::Evidence,
        ])
    );
}
