use a3s_runtime::contract::{
    ArtifactRef, HealthCheckKind, HealthProbe, IsolationLevel, MountKind, NetworkMode,
    ResourceControl, ResourceLimits, RestartPolicy, RuntimeActionRequest, RuntimeApplyRequest,
    RuntimeCapabilities, RuntimeExecRequest, RuntimeExecResult, RuntimeFeature, RuntimeHealthCheck,
    RuntimeHealthObservation, RuntimeHealthState, RuntimeInspection, RuntimeLogChunk,
    RuntimeLogQuery, RuntimeLogStream, RuntimeNetworkSpec, RuntimeObservation,
    RuntimeOutputArtifact, RuntimeOutputSpec, RuntimePort, RuntimeProcessSpec, RuntimeRemoval,
    RuntimeUnitClass, RuntimeUnitSpec, RuntimeUnitState, TransportProtocol,
};
use a3s_runtime::{
    verify_runtime_provider, FileRuntimeStateStore, ManagedRuntimeClient, ProviderId,
    RuntimeActionKind, RuntimeClient, RuntimeClientRegistry, RuntimeClock, RuntimeConformanceCase,
    RuntimeDriver, RuntimeError, RuntimeOperationLease, RuntimeProviderFactory, RuntimeResult,
    RuntimeStateReservation, RuntimeStateStore, RuntimeUnitRecord,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

const NOW: u64 = 1_000;
const IMAGE_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

#[derive(Debug)]
struct FixedClock;

impl RuntimeClock for FixedClock {
    fn now_ms(&self) -> u64 {
        NOW
    }
}

#[derive(Debug)]
struct ManualClock(AtomicU64);

impl ManualClock {
    fn new(now_ms: u64) -> Self {
        Self(AtomicU64::new(now_ms))
    }

    fn set(&self, now_ms: u64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }
}

impl RuntimeClock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct TestDriver {
    provider: ProviderId,
    capabilities: RuntimeCapabilities,
    state_directories: Mutex<Vec<tempfile::TempDir>>,
    apply_calls: AtomicUsize,
    inspect_calls: AtomicUsize,
    stop_calls: AtomicUsize,
    remove_calls: AtomicUsize,
    exec_calls: AtomicUsize,
    fail_next_apply: AtomicBool,
    missing_on_inspect: AtomicBool,
    substitute_identity: AtomicBool,
    unordered_logs: AtomicBool,
    return_starting_on_apply: AtomicBool,
    return_unknown_on_apply: AtomicBool,
    return_running_on_stop: AtomicBool,
    substitute_exec_spec: AtomicBool,
    hang_capabilities: AtomicBool,
    hang_apply: AtomicBool,
    hang_exec: AtomicBool,
}

impl TestDriver {
    fn new() -> Self {
        let provider = ProviderId::parse("test-runtime").unwrap();
        Self {
            capabilities: capabilities(provider.clone()),
            provider,
            state_directories: Mutex::new(Vec::new()),
            apply_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            stop_calls: AtomicUsize::new(0),
            remove_calls: AtomicUsize::new(0),
            exec_calls: AtomicUsize::new(0),
            fail_next_apply: AtomicBool::new(false),
            missing_on_inspect: AtomicBool::new(false),
            substitute_identity: AtomicBool::new(false),
            unordered_logs: AtomicBool::new(false),
            return_starting_on_apply: AtomicBool::new(false),
            return_unknown_on_apply: AtomicBool::new(false),
            return_running_on_stop: AtomicBool::new(false),
            substitute_exec_spec: AtomicBool::new(false),
            hang_capabilities: AtomicBool::new(false),
            hang_apply: AtomicBool::new(false),
            hang_exec: AtomicBool::new(false),
        }
    }

    fn client(self: &Arc<Self>) -> (ManagedRuntimeClient, Arc<FileRuntimeStateStore>) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        self.state_directories.lock().unwrap().push(directory);
        let store = Arc::new(FileRuntimeStateStore::new(path));
        let client =
            ManagedRuntimeClient::with_clock(store.clone(), self.clone(), Arc::new(FixedClock));
        (client, store)
    }
}

struct SignalingStateStore {
    inner: Arc<FileRuntimeStateStore>,
    lease_started: Arc<Notify>,
}

#[async_trait]
impl RuntimeStateStore for SignalingStateStore {
    async fn acquire_operation_lease(
        &self,
        unit_id: &str,
    ) -> RuntimeResult<Box<dyn RuntimeOperationLease>> {
        self.lease_started.notify_one();
        self.inner.acquire_operation_lease(unit_id).await
    }

    async fn reserve_apply(
        &self,
        request: &RuntimeApplyRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        self.inner.reserve_apply(request, now_ms).await
    }

    async fn reserve_action(
        &self,
        kind: RuntimeActionKind,
        request: &RuntimeActionRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        self.inner.reserve_action(kind, request, now_ms).await
    }

    async fn reserve_exec(
        &self,
        request: &RuntimeExecRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        self.inner.reserve_exec(request, now_ms).await
    }

    async fn load(&self, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord> {
        self.inner.load(unit_id).await
    }

    async fn load_request(
        &self,
        unit_id: &str,
        request_id: &str,
    ) -> RuntimeResult<a3s_runtime::RuntimeRequestReceipt> {
        self.inner.load_request(unit_id, request_id).await
    }

    async fn update_observation(
        &self,
        request_id: Option<&str>,
        observation: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeUnitRecord> {
        self.inner.update_observation(request_id, observation).await
    }

    async fn complete_removal(&self, removal: &RuntimeRemoval) -> RuntimeResult<RuntimeUnitRecord> {
        self.inner.complete_removal(removal).await
    }

    async fn complete_exec(&self, result: &RuntimeExecResult) -> RuntimeResult<RuntimeUnitRecord> {
        self.inner.complete_exec(result).await
    }
}

#[async_trait]
impl RuntimeDriver for TestDriver {
    fn provider_id(&self) -> &ProviderId {
        &self.provider
    }

    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        if self.hang_capabilities.load(Ordering::SeqCst) {
            return std::future::pending::<RuntimeResult<RuntimeCapabilities>>().await;
        }
        Ok(self.capabilities.clone())
    }

    async fn apply(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeObservation> {
        self.apply_calls.fetch_add(1, Ordering::SeqCst);
        if self.hang_apply.load(Ordering::SeqCst) {
            return std::future::pending::<RuntimeResult<RuntimeObservation>>().await;
        }
        if self.fail_next_apply.swap(false, Ordering::SeqCst) {
            return Err(RuntimeError::Transport("ambiguous apply".into()));
        }
        let mut observation = current.clone();
        observation.state = if self.return_unknown_on_apply.load(Ordering::SeqCst) {
            RuntimeUnitState::Unknown
        } else if self.return_starting_on_apply.load(Ordering::SeqCst) {
            RuntimeUnitState::Starting
        } else {
            match spec.class {
                RuntimeUnitClass::Task => RuntimeUnitState::Succeeded,
                RuntimeUnitClass::Service => RuntimeUnitState::Running,
            }
        };
        if self.substitute_identity.load(Ordering::SeqCst) {
            observation.unit_id = "substituted".into();
        }
        if observation.state != RuntimeUnitState::Unknown
            || !self.return_unknown_on_apply.load(Ordering::SeqCst)
        {
            let provider_identity = if current.state == RuntimeUnitState::Unknown {
                format!("provider/recovered/{}", spec.unit_id)
            } else {
                format!("provider/{}", spec.unit_id)
            };
            observation.provider_resource_id = Some(provider_identity);
            observation.provider_build = Some("test-driver/1".into());
        }
        observation.observed_at_ms = NOW + 1;
        observation.started_at_ms = Some(NOW);
        observation.finished_at_ms = observation.state.is_terminal().then_some(NOW + 1);
        observation.health = spec.health.as_ref().map(|_| RuntimeHealthObservation {
            state: RuntimeHealthState::Healthy,
            checked_at_ms: NOW + 1,
            message: None,
        });
        observation.outputs = if observation.state == RuntimeUnitState::Succeeded {
            spec.outputs
                .iter()
                .map(|expected| RuntimeOutputArtifact {
                    name: expected.name.clone(),
                    artifact: ArtifactRef {
                        media_type: expected.media_type.clone(),
                        ..artifact('b')
                    },
                    size_bytes: expected.max_bytes.min(4),
                })
                .collect()
        } else {
            Vec::new()
        };
        observation.provider_attestation =
            (spec.isolation == IsolationLevel::Confidential).then(|| artifact('c'));
        Ok(observation)
    }

    async fn inspect(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection> {
        self.inspect_calls.fetch_add(1, Ordering::SeqCst);
        if self.missing_on_inspect.load(Ordering::SeqCst) {
            return Ok(RuntimeInspection::NotFound {
                schema: RuntimeInspection::SCHEMA.into(),
                unit_id: unit.spec.unit_id.clone(),
                last_generation: Some(unit.spec.generation),
            });
        }
        let mut observation = unit.observation.clone();
        observation.observed_at_ms += 1;
        Ok(RuntimeInspection::Found {
            schema: RuntimeInspection::SCHEMA.into(),
            observation: Box::new(observation),
        })
    }

    async fn stop(
        &self,
        unit: &RuntimeUnitRecord,
        _request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeObservation> {
        self.stop_calls.fetch_add(1, Ordering::SeqCst);
        let mut observation = unit.observation.clone();
        observation.state = if self.return_running_on_stop.load(Ordering::SeqCst) {
            RuntimeUnitState::Running
        } else {
            RuntimeUnitState::Stopped
        };
        observation.observed_at_ms += 1;
        observation.finished_at_ms = observation
            .state
            .is_terminal()
            .then_some(observation.observed_at_ms);
        if observation.state.is_terminal() {
            observation.health = None;
        }
        observation.outputs.clear();
        Ok(observation)
    }

    async fn remove(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeRemoval> {
        self.remove_calls.fetch_add(1, Ordering::SeqCst);
        Ok(RuntimeRemoval {
            schema: RuntimeRemoval::SCHEMA.into(),
            request_id: request.request_id.clone(),
            unit_id: unit.spec.unit_id.clone(),
            generation: unit.spec.generation,
            removed_at_ms: NOW + 2,
            already_absent: false,
        })
    }

    async fn logs(
        &self,
        _unit: &RuntimeUnitRecord,
        _query: &RuntimeLogQuery,
    ) -> RuntimeResult<Vec<RuntimeLogChunk>> {
        let second_sequence = if self.unordered_logs.load(Ordering::SeqCst) {
            1
        } else {
            2
        };
        Ok(vec![
            RuntimeLogChunk {
                schema: RuntimeLogChunk::SCHEMA.into(),
                cursor: "cursor-1".into(),
                sequence: 1,
                observed_at_ms: NOW,
                stream: RuntimeLogStream::Stdout,
                data: "started\n".into(),
            },
            RuntimeLogChunk {
                schema: RuntimeLogChunk::SCHEMA.into(),
                cursor: "cursor-2".into(),
                sequence: second_sequence,
                observed_at_ms: NOW + 1,
                stream: RuntimeLogStream::Stdout,
                data: "ready\n".into(),
            },
        ])
    }

    async fn exec(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeExecRequest,
    ) -> RuntimeResult<RuntimeExecResult> {
        self.exec_calls.fetch_add(1, Ordering::SeqCst);
        if self.hang_exec.load(Ordering::SeqCst) {
            return std::future::pending::<RuntimeResult<RuntimeExecResult>>().await;
        }
        let mut observation = unit.observation.clone();
        if self.substitute_exec_spec.load(Ordering::SeqCst) {
            observation.spec_digest = digest('f');
        }
        Ok(RuntimeExecResult {
            schema: RuntimeExecResult::SCHEMA.into(),
            request_id: request.request_id.clone(),
            observation,
            exit_code: 0,
            stdout: "ok\n".into(),
            stderr: String::new(),
            truncated: false,
        })
    }
}

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn artifact(character: char) -> ArtifactRef {
    ArtifactRef {
        uri: format!(
            "oci://registry.example/a3s/demo@sha256:{}",
            character.to_string().repeat(64)
        ),
        digest: digest(character),
        media_type: IMAGE_MEDIA_TYPE.into(),
    }
}

fn resources(timeout: Option<u64>) -> ResourceLimits {
    ResourceLimits {
        cpu_millis: 500,
        memory_bytes: 128 * 1024 * 1024,
        pids: 128,
        ephemeral_storage_bytes: Some(1024 * 1024 * 1024),
        execution_timeout_ms: timeout,
    }
}

fn task(unit_id: &str, generation: u64) -> RuntimeUnitSpec {
    RuntimeUnitSpec {
        schema: RuntimeUnitSpec::SCHEMA.into(),
        unit_id: unit_id.into(),
        generation,
        class: RuntimeUnitClass::Task,
        artifact: artifact('a'),
        process: RuntimeProcessSpec {
            command: vec!["/bin/task".into()],
            args: vec![],
            working_directory: None,
            environment: BTreeMap::new(),
        },
        mounts: vec![],
        secrets: vec![],
        network: RuntimeNetworkSpec {
            mode: NetworkMode::Outbound,
            ports: vec![],
        },
        resources: resources(Some(60_000)),
        isolation: IsolationLevel::Container,
        health: None,
        restart: RestartPolicy::Never,
        outputs: vec![],
        semantics_profile_digest: None,
    }
}

fn service(unit_id: &str, generation: u64) -> RuntimeUnitSpec {
    let mut spec = task(unit_id, generation);
    spec.class = RuntimeUnitClass::Service;
    spec.resources = resources(None);
    spec.restart = RestartPolicy::Always;
    spec.network = RuntimeNetworkSpec {
        mode: NetworkMode::Service,
        ports: vec![RuntimePort {
            name: "http".into(),
            container_port: 8080,
            protocol: TransportProtocol::Tcp,
        }],
    };
    spec.health = Some(RuntimeHealthCheck {
        probe: HealthProbe::Http {
            port: "http".into(),
            path: "/health".into(),
            expected_statuses: vec![200],
        },
        interval_ms: 5_000,
        timeout_ms: 1_000,
        start_period_ms: 0,
        success_threshold: 1,
        failure_threshold: 3,
    });
    spec
}

fn apply(request_id: &str, spec: RuntimeUnitSpec) -> RuntimeApplyRequest {
    RuntimeApplyRequest {
        schema: RuntimeApplyRequest::SCHEMA.into(),
        request_id: request_id.into(),
        deadline_at_ms: Some(NOW + 60_000),
        spec,
    }
}

fn action(request_id: &str, unit_id: &str, generation: u64) -> RuntimeActionRequest {
    RuntimeActionRequest {
        schema: RuntimeActionRequest::SCHEMA.into(),
        request_id: request_id.into(),
        unit_id: unit_id.into(),
        generation,
        deadline_at_ms: Some(NOW + 60_000),
    }
}

fn capabilities(provider_id: ProviderId) -> RuntimeCapabilities {
    RuntimeCapabilities {
        schema: RuntimeCapabilities::SCHEMA.into(),
        provider_id,
        provider_build: "test-driver/1".into(),
        unit_classes: vec![RuntimeUnitClass::Task, RuntimeUnitClass::Service],
        artifact_media_types: vec![IMAGE_MEDIA_TYPE.into()],
        isolation_levels: vec![IsolationLevel::Container],
        network_modes: vec![
            NetworkMode::None,
            NetworkMode::Outbound,
            NetworkMode::Service,
        ],
        mount_kinds: vec![MountKind::Artifact, MountKind::Volume, MountKind::Tmpfs],
        health_check_kinds: vec![
            HealthCheckKind::Http,
            HealthCheckKind::Tcp,
            HealthCheckKind::Command,
        ],
        resource_controls: vec![
            ResourceControl::Cpu,
            ResourceControl::Memory,
            ResourceControl::Pids,
            ResourceControl::EphemeralStorage,
            ResourceControl::ExecutionTimeout,
        ],
        features: vec![
            RuntimeFeature::DurableIdentity,
            RuntimeFeature::Stop,
            RuntimeFeature::Remove,
            RuntimeFeature::Logs,
            RuntimeFeature::Exec,
            RuntimeFeature::OutputArtifacts,
        ],
    }
}

#[tokio::test]
async fn task_and_service_share_one_general_client() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();

    let task_observation = client
        .apply(&apply("apply-task", task("task-1", 1)))
        .await
        .unwrap();
    assert_eq!(task_observation.state, RuntimeUnitState::Succeeded);

    let service_request = apply("apply-service", service("service-1", 1));
    let service_observation = client.apply(&service_request).await.unwrap();
    assert_eq!(service_observation.state, RuntimeUnitState::Running);
    assert!(service_observation.converges(&service_request.spec));
    assert_eq!(
        client.apply(&service_request).await.unwrap(),
        service_observation
    );
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ambiguous_apply_is_reentered_with_the_same_identity() {
    let driver = Arc::new(TestDriver::new());
    driver.fail_next_apply.store(true, Ordering::SeqCst);
    let (client, store) = driver.client();
    let request = apply("apply-ambiguous", service("service-ambiguous", 1));

    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::Transport(_))
    ));
    let pending = store.load("service-ambiguous").await.unwrap();
    assert_eq!(pending.observation.state, RuntimeUnitState::Accepted);
    assert_eq!(
        store
            .load_request("service-ambiguous", "apply-ambiguous")
            .await
            .unwrap()
            .state,
        a3s_runtime::RuntimeRequestState::Pending
    );

    let recovered_client =
        ManagedRuntimeClient::with_clock(store, driver.clone(), Arc::new(FixedClock));
    let recovered = recovered_client.apply(&request).await.unwrap();
    assert_eq!(recovered.state, RuntimeUnitState::Running);
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn request_and_generation_conflicts_fail_closed() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();
    let first = apply("apply-1", service("service-versioned", 1));
    client.apply(&first).await.unwrap();

    let mut conflicting_request = first.clone();
    conflicting_request.deadline_at_ms = Some(NOW + 70_000);
    assert!(matches!(
        client.apply(&conflicting_request).await,
        Err(RuntimeError::RequestConflict { .. })
    ));

    let mut conflicting_generation = service("service-versioned", 1);
    conflicting_generation.artifact = artifact('b');
    assert!(matches!(
        client
            .apply(&apply("apply-conflict", conflicting_generation))
            .await,
        Err(RuntimeError::GenerationConflict { .. })
    ));

    client
        .apply(&apply("apply-2", service("service-versioned", 2)))
        .await
        .unwrap();
    assert!(matches!(
        client
            .apply(&apply("apply-stale", service("service-versioned", 1)))
            .await,
        Err(RuntimeError::StaleGeneration { .. })
    ));
}

#[tokio::test]
async fn stop_remove_and_absence_are_durable_and_idempotent() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();
    client
        .apply(&apply("apply-lifecycle", service("service-lifecycle", 1)))
        .await
        .unwrap();

    let stop = action("stop-lifecycle", "service-lifecycle", 1);
    let first_stop = client.stop(&stop).await.unwrap();
    assert!(matches!(
        &first_stop,
        RuntimeInspection::Found { observation, .. }
            if observation.state == RuntimeUnitState::Stopped
    ));
    assert_eq!(client.stop(&stop).await.unwrap(), first_stop);
    assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 1);

    let remove = action("remove-lifecycle", "service-lifecycle", 1);
    let first_remove = client.remove(&remove).await.unwrap();
    assert!(!first_remove.already_absent);
    assert_eq!(client.remove(&remove).await.unwrap(), first_remove);
    assert_eq!(driver.remove_calls.load(Ordering::SeqCst), 1);

    let second_remove = client
        .remove(&action("remove-lifecycle-again", "service-lifecycle", 1))
        .await
        .unwrap();
    assert!(second_remove.already_absent);
    assert_eq!(driver.remove_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        client.inspect("service-lifecycle").await.unwrap(),
        RuntimeInspection::NotFound {
            schema: RuntimeInspection::SCHEMA.into(),
            unit_id: "service-lifecycle".into(),
            last_generation: Some(1),
        }
    );
}

#[tokio::test]
async fn unsupported_capabilities_fail_before_state_or_provider_work() {
    let mut driver = TestDriver::new();
    driver.capabilities.unit_classes = vec![RuntimeUnitClass::Task];
    let driver = Arc::new(driver);
    let (client, store) = driver.client();
    assert!(matches!(
        client
            .apply(&apply("unsupported", service("unsupported-service", 1)))
            .await,
        Err(RuntimeError::UnsupportedCapabilities(_))
    ));
    assert!(matches!(
        store.load("unsupported-service").await,
        Err(RuntimeError::NotFound { .. })
    ));
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn optional_resource_controls_gate_only_workloads_that_request_them() {
    let mut driver = TestDriver::new();
    driver
        .capabilities
        .resource_controls
        .retain(|control| *control != ResourceControl::EphemeralStorage);
    let driver = Arc::new(driver);
    let (client, store) = driver.client();

    let mut without_quota = service("without-ephemeral-quota", 1);
    without_quota.resources.ephemeral_storage_bytes = None;
    client
        .apply(&apply("without-ephemeral-quota", without_quota))
        .await
        .expect("unrelated workload must remain supported");
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 1);

    assert!(matches!(
        client
            .apply(&apply(
                "with-ephemeral-quota",
                service("with-ephemeral-quota", 1),
            ))
            .await,
        Err(RuntimeError::UnsupportedCapabilities(missing))
            if missing == vec!["resource_control:EphemeralStorage"]
    ));
    assert!(matches!(
        store.load("with-ephemeral-quota").await,
        Err(RuntimeError::NotFound { .. })
    ));
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_identity_mismatch_fails_before_state_or_provider_work() {
    let mut driver = TestDriver::new();
    driver.capabilities.provider_id = ProviderId::parse("substituted-provider").unwrap();
    let driver = Arc::new(driver);
    let (client, store) = driver.client();

    assert!(matches!(
        client
            .apply(&apply("provider-mismatch", service("provider-mismatch", 1)))
            .await,
        Err(RuntimeError::Protocol(message)) if message.contains("reported capabilities")
    ));
    assert!(matches!(
        store.load("provider-mismatch").await,
        Err(RuntimeError::NotFound { .. })
    ));
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn output_and_confidential_capabilities_fail_closed_and_outputs_are_exact() {
    let mut output_task = task("output-task", 1);
    output_task.outputs = vec![RuntimeOutputSpec {
        name: "result".into(),
        path: "/outputs/result.json".into(),
        media_type: "application/json".into(),
        max_bytes: 16,
    }];

    let mut unsupported = TestDriver::new();
    unsupported
        .capabilities
        .features
        .retain(|feature| *feature != RuntimeFeature::OutputArtifacts);
    let unsupported = Arc::new(unsupported);
    let (client, store) = unsupported.client();
    assert!(matches!(
        client
            .apply(&apply("output-unsupported", output_task.clone()))
            .await,
        Err(RuntimeError::UnsupportedCapabilities(missing))
            if missing == vec!["feature:OutputArtifacts"]
    ));
    assert!(matches!(
        store.load("output-task").await,
        Err(RuntimeError::NotFound { .. })
    ));

    output_task.unit_id = "output-task-supported".into();
    let supported = Arc::new(TestDriver::new());
    let (client, _) = supported.client();
    let observation = client
        .apply(&apply("output-supported", output_task.clone()))
        .await
        .unwrap();
    assert_eq!(observation.outputs.len(), 1);
    assert_eq!(observation.outputs[0].name, "result");
    assert!(observation.outputs[0].size_bytes <= 16);
    let mut oversized = observation;
    oversized.outputs[0].size_bytes = 17;
    assert!(oversized.validate_against(&output_task).is_err());

    let mut confidential_driver = TestDriver::new();
    confidential_driver
        .capabilities
        .isolation_levels
        .push(IsolationLevel::Confidential);
    let confidential_driver = Arc::new(confidential_driver);
    let (client, store) = confidential_driver.client();
    let mut confidential = service("confidential-service", 1);
    confidential.isolation = IsolationLevel::Confidential;
    assert!(matches!(
        client
            .apply(&apply("confidential-unsupported", confidential))
            .await,
        Err(RuntimeError::UnsupportedCapabilities(missing))
            if missing == vec!["feature:Attestation"]
    ));
    assert!(matches!(
        store.load("confidential-service").await,
        Err(RuntimeError::NotFound { .. })
    ));

    let mut supported_confidential = TestDriver::new();
    supported_confidential
        .capabilities
        .isolation_levels
        .push(IsolationLevel::Confidential);
    supported_confidential
        .capabilities
        .features
        .push(RuntimeFeature::Attestation);
    let supported_confidential = Arc::new(supported_confidential);
    let (client, _) = supported_confidential.client();
    let mut confidential = service("confidential-supported", 1);
    confidential.isolation = IsolationLevel::Confidential;
    let observation = client
        .apply(&apply("confidential-supported", confidential))
        .await
        .unwrap();
    assert!(observation.provider_attestation.is_some());
}

#[tokio::test]
async fn apply_stop_and_exec_postconditions_fail_closed() {
    let driver = Arc::new(TestDriver::new());
    driver
        .return_starting_on_apply
        .store(true, Ordering::SeqCst);
    let (client, _) = driver.client();
    let spec = service("postconditions", 1);
    assert!(matches!(
        client
            .apply(&apply("postconditions-starting", spec.clone()))
            .await,
        Err(RuntimeError::Protocol(message)) if message.contains("invalid Service result")
    ));

    driver
        .return_starting_on_apply
        .store(false, Ordering::SeqCst);
    client
        .apply(&apply("postconditions-running", spec))
        .await
        .unwrap();
    driver.return_running_on_stop.store(true, Ordering::SeqCst);
    let stop = action("postconditions-stop", "postconditions", 1);
    assert!(matches!(
        client.stop(&stop).await,
        Err(RuntimeError::Protocol(message)) if message.contains("nonterminal state")
    ));

    driver.return_running_on_stop.store(false, Ordering::SeqCst);
    client.stop(&stop).await.unwrap();
    assert!(matches!(
        client
            .exec(&RuntimeExecRequest {
                schema: RuntimeExecRequest::SCHEMA.into(),
                request_id: "exec-stopped".into(),
                unit_id: "postconditions".into(),
                generation: 1,
                command: vec!["/bin/true".into()],
                timeout_ms: 1_000,
                deadline_at_ms: None,
            })
            .await,
        Err(RuntimeError::InvalidRequest(message)) if message.contains("running unit")
    ));
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 0);

    let unknown_driver = Arc::new(TestDriver::new());
    unknown_driver
        .return_unknown_on_apply
        .store(true, Ordering::SeqCst);
    let (unknown_client, _) = unknown_driver.client();
    assert!(matches!(
        unknown_client
            .apply(&apply(
                "postconditions-unknown",
                service("postconditions-unknown", 1),
            ))
            .await,
        Err(RuntimeError::Protocol(message))
            if message.contains("without provider identity")
    ));
}

#[tokio::test]
async fn exec_result_must_bind_the_complete_unit_spec() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();
    client
        .apply(&apply("exec-binding-apply", service("exec-binding", 1)))
        .await
        .unwrap();
    driver.substitute_exec_spec.store(true, Ordering::SeqCst);

    assert!(matches!(
        client
            .exec(&RuntimeExecRequest {
                schema: RuntimeExecRequest::SCHEMA.into(),
                request_id: "exec-binding-request".into(),
                unit_id: "exec-binding".into(),
                generation: 1,
                command: vec!["/bin/true".into()],
                timeout_ms: 1_000,
                deadline_at_ms: None,
            })
            .await,
        Err(RuntimeError::Protocol(message))
            if message.contains("does not match the unit specification")
    ));
}

#[tokio::test]
async fn completed_exec_replays_durably_after_client_restart_and_rejects_conflicts() {
    let driver = Arc::new(TestDriver::new());
    let (client, store) = driver.client();
    client
        .apply(&apply("exec-restart-apply", service("exec-restart", 1)))
        .await
        .unwrap();
    let request = RuntimeExecRequest {
        schema: RuntimeExecRequest::SCHEMA.into(),
        request_id: "exec-restart-request".into(),
        unit_id: "exec-restart".into(),
        generation: 1,
        command: vec!["/bin/true".into()],
        timeout_ms: 1_000,
        deadline_at_ms: None,
    };
    let expected = client.exec(&request).await.unwrap();
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .load_request("exec-restart", "exec-restart-request")
            .await
            .unwrap()
            .exec_result,
        Some(expected.clone())
    );

    let restarted = ManagedRuntimeClient::with_clock(store, driver.clone(), Arc::new(FixedClock));
    assert_eq!(restarted.exec(&request).await.unwrap(), expected);
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 1);

    let mut conflicting = request;
    conflicting.command = vec!["/bin/false".into()];
    assert!(matches!(
        restarted.exec(&conflicting).await,
        Err(RuntimeError::RequestConflict { .. })
    ));
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn completed_mutations_replay_after_deadline_provider_loss_and_removal() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let driver = Arc::new(TestDriver::new());
    let clock = Arc::new(ManualClock::new(NOW));
    let client = ManagedRuntimeClient::with_clock(store.clone(), driver.clone(), clock.clone());
    let deadline = NOW + 1_000;

    let mut apply_request = apply("durable-replay-apply", service("durable-replay", 1));
    apply_request.deadline_at_ms = Some(deadline);
    let applied = client.apply(&apply_request).await.unwrap();

    let exec_request = RuntimeExecRequest {
        schema: RuntimeExecRequest::SCHEMA.into(),
        request_id: "durable-replay-exec".into(),
        unit_id: "durable-replay".into(),
        generation: 1,
        command: vec!["/bin/true".into()],
        timeout_ms: 500,
        deadline_at_ms: Some(deadline),
    };
    let executed = client.exec(&exec_request).await.unwrap();

    let mut stop_request = action("durable-replay-stop", "durable-replay", 1);
    stop_request.deadline_at_ms = Some(deadline);
    let stopped = client.stop(&stop_request).await.unwrap();

    let mut remove_request = action("durable-replay-remove", "durable-replay", 1);
    remove_request.deadline_at_ms = Some(deadline);
    let removed = client.remove(&remove_request).await.unwrap();

    clock.set(deadline);
    driver.hang_capabilities.store(true, Ordering::SeqCst);
    let restarted = ManagedRuntimeClient::with_clock(store, driver.clone(), clock);

    assert_eq!(restarted.apply(&apply_request).await.unwrap(), applied);
    assert_eq!(restarted.exec(&exec_request).await.unwrap(), executed);
    assert_eq!(restarted.stop(&stop_request).await.unwrap(), stopped);
    assert_eq!(restarted.remove(&remove_request).await.unwrap(), removed);
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 1);
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 1);
    assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(driver.remove_calls.load(Ordering::SeqCst), 1);

    let mut conflicting = apply_request;
    conflicting.spec.process.command = vec!["/bin/false".into()];
    assert!(matches!(
        restarted.apply(&conflicting).await,
        Err(RuntimeError::RequestConflict { .. })
    ));
}

#[tokio::test]
async fn expired_pending_request_remains_pending_and_is_not_redispatched() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let driver = Arc::new(TestDriver::new());
    let clock = Arc::new(ManualClock::new(NOW));
    let client = ManagedRuntimeClient::with_clock(store.clone(), driver.clone(), clock.clone());
    let mut request = apply("pending-expiry", service("pending-expiry", 1));
    request.deadline_at_ms = Some(NOW + 10);
    driver.hang_apply.store(true, Ordering::SeqCst);

    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::DeadlineExceeded(message)) if message.contains("provider apply")
    ));
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .load_request("pending-expiry", "pending-expiry")
            .await
            .unwrap()
            .state,
        a3s_runtime::RuntimeRequestState::Pending
    );

    clock.set(NOW + 10);
    driver.hang_apply.store(false, Ordering::SeqCst);
    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::DeadlineExceeded(_))
    ));
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .load_request("pending-expiry", "pending-expiry")
            .await
            .unwrap()
            .state,
        a3s_runtime::RuntimeRequestState::Pending
    );
}

#[test]
fn top_level_log_exec_and_inspection_records_require_current_schemas() {
    let mut query = RuntimeLogQuery {
        schema: RuntimeLogQuery::SCHEMA.into(),
        unit_id: "schema-unit".into(),
        generation: 1,
        cursor: None,
        limit: 1,
        stream: None,
    };
    query.validate().unwrap();
    query.schema = "a3s.runtime.log-query.v0".into();
    assert!(query.validate().is_err());

    let mut chunk = RuntimeLogChunk {
        schema: RuntimeLogChunk::SCHEMA.into(),
        cursor: "cursor".into(),
        sequence: 1,
        observed_at_ms: NOW,
        stream: RuntimeLogStream::Stdout,
        data: String::new(),
    };
    chunk.validate().unwrap();
    chunk.schema.clear();
    assert!(chunk.validate().is_err());

    let mut exec = RuntimeExecRequest {
        schema: RuntimeExecRequest::SCHEMA.into(),
        request_id: "schema-exec".into(),
        unit_id: "schema-unit".into(),
        generation: 1,
        command: vec!["/bin/true".into()],
        timeout_ms: 1,
        deadline_at_ms: None,
    };
    exec.validate().unwrap();
    exec.schema = "future".into();
    assert!(exec.validate().is_err());

    let inspection = RuntimeInspection::NotFound {
        schema: "future".into(),
        unit_id: "schema-unit".into(),
        last_generation: None,
    };
    assert!(inspection.validate().is_err());

    assert!(
        serde_json::from_value::<RuntimeLogQuery>(serde_json::json!({
            "unit_id": "schema-unit",
            "generation": 1,
            "cursor": null,
            "limit": 1,
            "stream": null
        }))
        .is_err()
    );
}

#[tokio::test]
async fn provider_identity_substitution_and_unordered_logs_are_rejected() {
    let driver = Arc::new(TestDriver::new());
    driver.substitute_identity.store(true, Ordering::SeqCst);
    let (client, _) = driver.client();
    assert!(matches!(
        client
            .apply(&apply("substitute", service("identity-service", 1)))
            .await,
        Err(RuntimeError::Protocol(_))
    ));

    driver.substitute_identity.store(false, Ordering::SeqCst);
    client
        .apply(&apply("identity-retry", service("identity-service", 1)))
        .await
        .unwrap();
    driver.unordered_logs.store(true, Ordering::SeqCst);
    assert!(matches!(
        client
            .logs(&RuntimeLogQuery {
                schema: RuntimeLogQuery::SCHEMA.into(),
                unit_id: "identity-service".into(),
                generation: 1,
                cursor: None,
                limit: 100,
                stream: None,
            })
            .await,
        Err(RuntimeError::Protocol(_))
    ));
}

#[tokio::test]
async fn log_and_exec_capabilities_use_the_active_generation() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();
    client
        .apply(&apply("apply-tools", service("service-tools", 1)))
        .await
        .unwrap();
    let logs = client
        .logs(&RuntimeLogQuery {
            schema: RuntimeLogQuery::SCHEMA.into(),
            unit_id: "service-tools".into(),
            generation: 1,
            cursor: None,
            limit: 100,
            stream: None,
        })
        .await
        .unwrap();
    assert_eq!(logs.len(), 2);

    let result = client
        .exec(&RuntimeExecRequest {
            schema: RuntimeExecRequest::SCHEMA.into(),
            request_id: "exec-tools".into(),
            unit_id: "service-tools".into(),
            generation: 1,
            command: vec!["/bin/true".into()],
            timeout_ms: 1_000,
            deadline_at_ms: None,
        })
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        client
            .exec(&RuntimeExecRequest {
                schema: RuntimeExecRequest::SCHEMA.into(),
                request_id: "exec-tools".into(),
                unit_id: "service-tools".into(),
                generation: 1,
                command: vec!["/bin/true".into()],
                timeout_ms: 1_000,
                deadline_at_ms: None,
            })
            .await
            .unwrap(),
        result
    );
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 1);

    assert!(matches!(
        client
            .logs(&RuntimeLogQuery {
                schema: RuntimeLogQuery::SCHEMA.into(),
                unit_id: "service-tools".into(),
                generation: 2,
                cursor: None,
                limit: 100,
                stream: None,
            })
            .await,
        Err(RuntimeError::GenerationConflict { .. })
    ));
}

#[tokio::test]
async fn expired_requests_never_reserve_or_dispatch() {
    let driver = Arc::new(TestDriver::new());
    let (client, store) = driver.client();
    let mut request = apply("expired", service("expired-service", 1));
    request.deadline_at_ms = Some(NOW);
    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::DeadlineExceeded(_))
    ));
    assert!(matches!(
        store.load("expired-service").await,
        Err(RuntimeError::NotFound { .. })
    ));
}

#[tokio::test]
async fn file_operation_leases_serialize_one_unit_but_not_different_units() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let first = store.acquire_operation_lease("lease-unit").await.unwrap();

    assert!(tokio::time::timeout(
        Duration::from_millis(20),
        store.acquire_operation_lease("lease-unit"),
    )
    .await
    .is_err());
    let different = tokio::time::timeout(
        Duration::from_millis(100),
        store.acquire_operation_lease("different-unit"),
    )
    .await
    .expect("a different unit must not wait behind the held lease")
    .unwrap();

    drop(different);
    drop(first);
}

#[tokio::test]
async fn subprocess_operation_lease_helper() {
    let Ok(root) = std::env::var("A3S_RUNTIME_LEASE_HELPER_ROOT") else {
        return;
    };
    let unit_id = std::env::var("A3S_RUNTIME_LEASE_HELPER_UNIT").unwrap();
    let ready = std::env::var("A3S_RUNTIME_LEASE_HELPER_READY").unwrap();
    let release = std::env::var("A3S_RUNTIME_LEASE_HELPER_RELEASE").unwrap();
    let store = FileRuntimeStateStore::new(root);
    let _lease = store.acquire_operation_lease(&unit_id).await.unwrap();
    std::fs::write(&ready, b"ready").unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !std::path::Path::new(&release).is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("parent did not release the subprocess lease");
}

#[tokio::test]
async fn file_operation_lease_serializes_independent_processes() {
    use std::process::Command;

    let directory = tempfile::tempdir().unwrap();
    let ready = directory.path().join("helper.ready");
    let release = directory.path().join("helper.release");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("subprocess_operation_lease_helper")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env("A3S_RUNTIME_LEASE_HELPER_ROOT", directory.path())
        .env("A3S_RUNTIME_LEASE_HELPER_UNIT", "cross-process-unit")
        .env("A3S_RUNTIME_LEASE_HELPER_READY", &ready)
        .env("A3S_RUNTIME_LEASE_HELPER_RELEASE", &release)
        .spawn()
        .unwrap();

    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("subprocess did not acquire the operation lease");

    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let waiting_store = store.clone();
    let waiter = tokio::spawn(async move {
        waiting_store
            .acquire_operation_lease("cross-process-unit")
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiter.is_finished());
    let different = tokio::time::timeout(
        Duration::from_millis(100),
        store.acquire_operation_lease("cross-process-other"),
    )
    .await
    .expect("different-unit cross-process lease was blocked")
    .unwrap();
    drop(different);

    std::fs::write(&release, b"release").unwrap();
    let acquired = tokio::time::timeout(Duration::from_secs(10), waiter)
        .await
        .expect("same-unit waiter did not acquire after subprocess release")
        .unwrap()
        .unwrap();
    drop(acquired);
    assert!(child.wait().unwrap().success());
}

#[tokio::test]
async fn deadline_is_rechecked_after_the_operation_lease_wait() {
    let directory = tempfile::tempdir().unwrap();
    let file_store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let held = file_store
        .acquire_operation_lease("post-lease-deadline")
        .await
        .unwrap();
    let lease_started = Arc::new(Notify::new());
    let state: Arc<dyn RuntimeStateStore> = Arc::new(SignalingStateStore {
        inner: file_store.clone(),
        lease_started: lease_started.clone(),
    });
    let driver = Arc::new(TestDriver::new());
    let clock = Arc::new(ManualClock::new(NOW));
    let client = ManagedRuntimeClient::with_clock(state, driver.clone(), clock.clone());
    let mut request = apply("post-lease-deadline", service("post-lease-deadline", 1));
    request.deadline_at_ms = Some(NOW + 1_000);

    let operation = tokio::spawn(async move { client.apply(&request).await });
    lease_started.notified().await;
    clock.set(NOW + 1_000);
    drop(held);

    assert!(matches!(
        operation.await.unwrap(),
        Err(RuntimeError::DeadlineExceeded(message))
            if message.contains("before provider dispatch")
    ));
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        file_store.load("post-lease-deadline").await,
        Err(RuntimeError::NotFound { .. })
    ));
}

#[tokio::test]
async fn deadlines_bound_capability_queries_and_provider_dispatch() {
    let capability_driver = Arc::new(TestDriver::new());
    capability_driver
        .hang_capabilities
        .store(true, Ordering::SeqCst);
    let (client, store) = capability_driver.client();
    let mut request = apply("capability-timeout", service("capability-timeout", 1));
    request.deadline_at_ms = Some(NOW + 10);
    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::DeadlineExceeded(message))
            if message.contains("capability query")
    ));
    assert!(matches!(
        store.load("capability-timeout").await,
        Err(RuntimeError::NotFound { .. })
    ));
    assert_eq!(capability_driver.apply_calls.load(Ordering::SeqCst), 0);

    let provider_driver = Arc::new(TestDriver::new());
    provider_driver.hang_apply.store(true, Ordering::SeqCst);
    let (client, store) = provider_driver.client();
    let mut request = apply("provider-timeout", service("provider-timeout", 1));
    request.deadline_at_ms = Some(NOW + 10);
    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::DeadlineExceeded(message)) if message.contains("provider apply")
    ));
    let pending = store
        .load_request("provider-timeout", "provider-timeout")
        .await
        .unwrap();
    assert_eq!(pending.state, a3s_runtime::RuntimeRequestState::Pending);
    assert_eq!(provider_driver.apply_calls.load(Ordering::SeqCst), 1);

    provider_driver.hang_apply.store(false, Ordering::SeqCst);
    assert_eq!(
        client.apply(&request).await.unwrap().state,
        RuntimeUnitState::Running
    );
    assert_eq!(provider_driver.apply_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn exec_uses_the_smaller_relative_or_absolute_deadline() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();
    client
        .apply(&apply("exec-deadline-apply", service("exec-deadline", 1)))
        .await
        .unwrap();
    driver.hang_exec.store(true, Ordering::SeqCst);

    assert!(matches!(
        client
            .exec(&RuntimeExecRequest {
                schema: RuntimeExecRequest::SCHEMA.into(),
                request_id: "exec-relative-deadline".into(),
                unit_id: "exec-deadline".into(),
                generation: 1,
                command: vec!["/bin/true".into()],
                timeout_ms: 10,
                deadline_at_ms: Some(NOW + 1_000),
            })
            .await,
        Err(RuntimeError::DeadlineExceeded(message)) if message.contains("provider exec")
    ));
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 1);

    assert!(matches!(
        client
            .exec(&RuntimeExecRequest {
                schema: RuntimeExecRequest::SCHEMA.into(),
                request_id: "exec-absolute-deadline".into(),
                unit_id: "exec-deadline".into(),
                generation: 1,
                command: vec!["/bin/true".into()],
                timeout_ms: 1_000,
                deadline_at_ms: Some(NOW),
            })
            .await,
        Err(RuntimeError::DeadlineExceeded(_))
    ));
    assert_eq!(driver.exec_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_loss_becomes_a_durable_unknown_observation() {
    let driver = Arc::new(TestDriver::new());
    let (client, store) = driver.client();
    let running = client
        .apply(&apply("apply-loss", service("service-loss", 1)))
        .await
        .unwrap();
    driver.missing_on_inspect.store(true, Ordering::SeqCst);

    let inspection = client.inspect("service-loss").await.unwrap();
    let RuntimeInspection::Found { observation, .. } = inspection else {
        panic!("a previously observed provider unit must not become definitively absent");
    };
    assert_eq!(observation.state, RuntimeUnitState::Unknown);
    assert_eq!(
        observation.provider_resource_id,
        running.provider_resource_id
    );
    assert!(observation.observed_at_ms >= running.observed_at_ms);
    assert_eq!(
        store.load("service-loss").await.unwrap().observation,
        *observation
    );
}

#[tokio::test]
async fn same_generation_apply_recovers_a_lost_provider_unit_once() {
    let driver = Arc::new(TestDriver::new());
    let (client, store) = driver.client();
    let spec = service("service-recovery", 1);
    let running = client
        .apply(&apply("apply-recovery", spec.clone()))
        .await
        .unwrap();

    driver.missing_on_inspect.store(true, Ordering::SeqCst);
    let RuntimeInspection::Found { observation, .. } =
        client.inspect("service-recovery").await.unwrap()
    else {
        panic!("provider loss must first become a durable unknown observation");
    };
    assert_eq!(observation.state, RuntimeUnitState::Unknown);

    driver.missing_on_inspect.store(false, Ordering::SeqCst);
    let recovery = apply("reapply-recovery", spec);
    let recovered = client.apply(&recovery).await.unwrap();
    assert_eq!(recovered.state, RuntimeUnitState::Running);
    assert_ne!(recovered.provider_resource_id, running.provider_resource_id);
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 2);

    assert_eq!(client.apply(&recovery).await.unwrap(), recovered);
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        store.load("service-recovery").await.unwrap().observation,
        recovered
    );
}

#[tokio::test]
async fn terminal_observations_are_immutable() {
    let driver = Arc::new(TestDriver::new());
    let (client, store) = driver.client();
    let terminal = client
        .apply(&apply("apply-terminal", task("terminal-task", 1)))
        .await
        .unwrap();
    assert_eq!(terminal.state, RuntimeUnitState::Succeeded);

    let mut changed = terminal.clone();
    changed.observed_at_ms += 1;
    assert!(matches!(
        store.update_observation(None, &changed).await,
        Err(RuntimeError::Protocol(_))
    ));
    assert_eq!(
        store.load("terminal-task").await.unwrap().observation,
        terminal
    );
}

#[tokio::test]
async fn concurrent_file_reservations_preserve_every_request() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let expected = service("concurrent-service", 1);
    let mut tasks = Vec::new();
    for index in 0..32 {
        let store = store.clone();
        let request = apply(&format!("apply-{index}"), expected.clone());
        tasks.push(tokio::spawn(async move {
            store.reserve_apply(&request, NOW).await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }

    let record = store.load("concurrent-service").await.unwrap();
    assert_eq!(record.spec, expected);
    for index in 0..32 {
        assert_eq!(
            store
                .load_request("concurrent-service", &format!("apply-{index}"))
                .await
                .unwrap()
                .state,
            a3s_runtime::RuntimeRequestState::Pending
        );
    }
}

#[tokio::test]
async fn request_journal_preserves_more_than_ten_thousand_exact_replays() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let driver = Arc::new(TestDriver::new());
    let client =
        ManagedRuntimeClient::with_clock(store.clone(), driver.clone(), Arc::new(FixedClock));
    let spec = service("journal-capacity", 1);
    client
        .apply(&apply("journal-request-0", spec.clone()))
        .await
        .unwrap();
    for index in 1..=10_000 {
        client
            .apply(&apply(&format!("journal-request-{index}"), spec.clone()))
            .await
            .unwrap();
    }

    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .load_request("journal-capacity", "journal-request-10000")
            .await
            .unwrap()
            .state,
        a3s_runtime::RuntimeRequestState::Completed
    );
    let unit_directory = std::fs::read_dir(directory.path().join("units"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read_dir(unit_directory.join("requests"))
            .unwrap()
            .count(),
        10_001
    );
}

struct CountingFactory {
    provider: ProviderId,
    creates: AtomicUsize,
    client: Arc<dyn RuntimeClient>,
}

#[async_trait]
impl RuntimeProviderFactory for CountingFactory {
    fn provider_id(&self) -> &ProviderId {
        &self.provider
    }

    async fn create(&self) -> RuntimeResult<Arc<dyn RuntimeClient>> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(self.client.clone())
    }
}

#[tokio::test]
async fn provider_registry_never_falls_back_or_replaces_a_factory() {
    let mut driver = TestDriver::new();
    driver.provider = ProviderId::parse("explicit-provider").unwrap();
    driver.capabilities.provider_id = driver.provider.clone();
    let driver = Arc::new(driver);
    let (client, _) = driver.client();
    let factory = Arc::new(CountingFactory {
        provider: ProviderId::parse("explicit-provider").unwrap(),
        creates: AtomicUsize::new(0),
        client: Arc::new(client),
    });
    let mut registry = RuntimeClientRegistry::new();
    registry.register(factory.clone()).unwrap();

    let missing = ProviderId::parse("missing-provider").unwrap();
    assert!(matches!(
        registry.connect(&missing).await,
        Err(RuntimeError::ProviderUnavailable(_))
    ));
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);
    assert!(registry.register(factory.clone()).is_err());
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);

    registry.connect(factory.provider_id()).await.unwrap();
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_registry_rejects_a_factory_client_identity_mismatch() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();
    let factory = Arc::new(CountingFactory {
        provider: ProviderId::parse("registered-provider").unwrap(),
        creates: AtomicUsize::new(0),
        client: Arc::new(client),
    });
    let mut registry = RuntimeClientRegistry::new();
    registry.register(factory.clone()).unwrap();

    assert!(matches!(
        registry.connect(factory.provider_id()).await,
        Err(RuntimeError::Protocol(message)) if message.contains("created client reporting")
    ));
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn file_state_store_rejects_symbolic_link_boundaries() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let linked_root = directory.path().join("linked-root");
    symlink(target.path(), &linked_root).unwrap();
    let linked_store = FileRuntimeStateStore::new(&linked_root);
    assert!(linked_store.load("linked-unit").await.is_err());

    let root = directory.path().join("state");
    std::fs::create_dir(&root).unwrap();
    symlink(target.path(), root.join("units")).unwrap();
    let store = FileRuntimeStateStore::new(&root);
    assert!(store
        .reserve_apply(&apply("linked-units", task("linked-unit", 1)), NOW)
        .await
        .is_err());

    std::fs::remove_file(root.join("units")).unwrap();
    store
        .reserve_apply(&apply("secure", task("secure-unit", 1)), NOW)
        .await
        .unwrap();
    let lock_path = root
        .join("locks")
        .join(format!("{:x}.lock", Sha256::digest(b"secure-unit")));
    std::fs::remove_file(&lock_path).unwrap();
    let lock_target = root.join("lock-target");
    std::fs::write(&lock_target, b"do not follow").unwrap();
    symlink(&lock_target, &lock_path).unwrap();
    assert!(matches!(
        store.load("secure-unit").await,
        Err(RuntimeError::Protocol(_))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn request_journal_layout_is_versioned_owner_only_and_fail_closed() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    let driver = Arc::new(TestDriver::new());
    let client = ManagedRuntimeClient::with_clock(store.clone(), driver, Arc::new(FixedClock));
    client
        .apply(&apply("layout-request", service("layout-unit", 1)))
        .await
        .unwrap();

    let unit_directory = std::fs::read_dir(directory.path().join("units"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let record_path = unit_directory.join("record.json");
    let receipt_path = std::fs::read_dir(unit_directory.join("requests"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    assert_eq!(record["schema"], a3s_runtime::RuntimeUnitRecord::SCHEMA);
    assert!(record.get("requests").is_none());
    assert_eq!(
        receipt["schema"],
        a3s_runtime::RuntimeRequestReceipt::SCHEMA
    );
    assert_eq!(
        std::fs::metadata(&unit_directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&record_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&receipt_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let mut unknown_field = receipt;
    unknown_field["unexpected"] = serde_json::json!(true);
    std::fs::write(&receipt_path, serde_json::to_vec(&unknown_field).unwrap()).unwrap();
    assert!(matches!(
        store.load_request("layout-unit", "layout-request").await,
        Err(RuntimeError::Protocol(_))
    ));

    let legacy = tempfile::tempdir().unwrap();
    let units = legacy.path().join("units");
    std::fs::create_dir(&units).unwrap();
    let key = format!("{:x}", Sha256::digest(b"legacy-unit"));
    let legacy_record = units.join(format!("{key}.json"));
    std::fs::write(&legacy_record, b"{}").unwrap();
    std::fs::set_permissions(&legacy_record, std::fs::Permissions::from_mode(0o600)).unwrap();
    let legacy_store = FileRuntimeStateStore::new(legacy.path());
    assert!(matches!(
        legacy_store.load("legacy-unit").await,
        Err(RuntimeError::Protocol(message)) if message.contains("explicit migration")
    ));
}

#[tokio::test]
async fn exported_conformance_suite_exercises_task_and_service_lifecycles() {
    let driver = Arc::new(TestDriver::new());
    let (client, _) = driver.client();
    let case = RuntimeConformanceCase {
        task_apply: apply("conformance-task-apply", task("conformance-task", 1)),
        task_remove: action("conformance-task-remove", "conformance-task", 1),
        service_apply: apply(
            "conformance-service-apply",
            service("conformance-service", 1),
        ),
        service_stop: action("conformance-service-stop", "conformance-service", 1),
        service_remove: action("conformance-service-remove", "conformance-service", 1),
    };

    let report = verify_runtime_provider(&client, &case).await.unwrap();
    assert_eq!(report.task.state, RuntimeUnitState::Succeeded);
    assert_eq!(report.service.state, RuntimeUnitState::Running);
    assert_eq!(report.stopped_service.state, RuntimeUnitState::Stopped);
    assert_eq!(driver.apply_calls.load(Ordering::SeqCst), 2);
    assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(driver.remove_calls.load(Ordering::SeqCst), 2);
}
