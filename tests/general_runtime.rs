use a3s_runtime::contract::{
    ArtifactRef, HealthCheckKind, HealthProbe, IsolationLevel, MountKind, NetworkMode,
    ResourceControl, ResourceLimits, RestartPolicy, RuntimeActionRequest, RuntimeApplyRequest,
    RuntimeCapabilities, RuntimeExecRequest, RuntimeExecResult, RuntimeFeature, RuntimeHealthCheck,
    RuntimeHealthObservation, RuntimeHealthState, RuntimeInspection, RuntimeLogChunk,
    RuntimeLogQuery, RuntimeLogStream, RuntimeNetworkSpec, RuntimeObservation, RuntimePort,
    RuntimeProcessSpec, RuntimeRemoval, RuntimeUnitClass, RuntimeUnitSpec, RuntimeUnitState,
    TransportProtocol,
};
use a3s_runtime::{
    verify_runtime_provider, FileRuntimeStateStore, ManagedRuntimeClient, ProviderId,
    RuntimeClient, RuntimeClientRegistry, RuntimeClock, RuntimeConformanceCase, RuntimeDriver,
    RuntimeError, RuntimeProviderFactory, RuntimeResult, RuntimeStateStore, RuntimeUnitRecord,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const NOW: u64 = 1_000;
const IMAGE_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

#[derive(Debug)]
struct FixedClock;

impl RuntimeClock for FixedClock {
    fn now_ms(&self) -> u64 {
        NOW
    }
}

struct TestDriver {
    capabilities: RuntimeCapabilities,
    apply_calls: AtomicUsize,
    inspect_calls: AtomicUsize,
    stop_calls: AtomicUsize,
    remove_calls: AtomicUsize,
    fail_next_apply: AtomicBool,
    missing_on_inspect: AtomicBool,
    substitute_identity: AtomicBool,
    unordered_logs: AtomicBool,
}

impl TestDriver {
    fn new() -> Self {
        Self {
            capabilities: capabilities(),
            apply_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            stop_calls: AtomicUsize::new(0),
            remove_calls: AtomicUsize::new(0),
            fail_next_apply: AtomicBool::new(false),
            missing_on_inspect: AtomicBool::new(false),
            substitute_identity: AtomicBool::new(false),
            unordered_logs: AtomicBool::new(false),
        }
    }

    fn client(self: &Arc<Self>) -> (ManagedRuntimeClient, Arc<FileRuntimeStateStore>) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.keep();
        let store = Arc::new(FileRuntimeStateStore::new(path));
        let client =
            ManagedRuntimeClient::with_clock(store.clone(), self.clone(), Arc::new(FixedClock));
        (client, store)
    }
}

#[async_trait]
impl RuntimeDriver for TestDriver {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        Ok(self.capabilities.clone())
    }

    async fn apply(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeObservation> {
        self.apply_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_next_apply.swap(false, Ordering::SeqCst) {
            return Err(RuntimeError::Transport("ambiguous apply".into()));
        }
        let mut observation = current.clone();
        observation.state = match spec.class {
            RuntimeUnitClass::Task => RuntimeUnitState::Succeeded,
            RuntimeUnitClass::Service => RuntimeUnitState::Running,
        };
        if self.substitute_identity.load(Ordering::SeqCst) {
            observation.unit_id = "substituted".into();
        }
        observation.provider_resource_id = Some(format!("provider/{}", spec.unit_id));
        observation.provider_build = Some("test-driver/1".into());
        observation.observed_at_ms = NOW + 1;
        observation.started_at_ms = Some(NOW);
        observation.finished_at_ms = (spec.class == RuntimeUnitClass::Task).then_some(NOW + 1);
        observation.health = spec.health.as_ref().map(|_| RuntimeHealthObservation {
            state: RuntimeHealthState::Healthy,
            checked_at_ms: NOW + 1,
            message: None,
        });
        Ok(observation)
    }

    async fn inspect(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection> {
        self.inspect_calls.fetch_add(1, Ordering::SeqCst);
        if self.missing_on_inspect.load(Ordering::SeqCst) {
            return Ok(RuntimeInspection::NotFound {
                unit_id: unit.spec.unit_id.clone(),
                last_generation: Some(unit.spec.generation),
            });
        }
        let mut observation = unit.observation.clone();
        observation.observed_at_ms += 1;
        Ok(RuntimeInspection::Found {
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
        observation.state = RuntimeUnitState::Stopped;
        observation.observed_at_ms += 1;
        observation.finished_at_ms = Some(observation.observed_at_ms);
        observation.health = None;
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
                cursor: "cursor-1".into(),
                sequence: 1,
                observed_at_ms: NOW,
                stream: RuntimeLogStream::Stdout,
                data: "started\n".into(),
            },
            RuntimeLogChunk {
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
        Ok(RuntimeExecResult {
            request_id: request.request_id.clone(),
            observation: unit.observation.clone(),
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
        ephemeral_storage_bytes: 1024 * 1024 * 1024,
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

fn capabilities() -> RuntimeCapabilities {
    RuntimeCapabilities {
        schema: RuntimeCapabilities::SCHEMA.into(),
        provider_id: "test-runtime".into(),
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
        pending.receipt("apply-ambiguous").unwrap().state,
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
        RuntimeInspection::Found { observation }
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
            request_id: "exec-tools".into(),
            unit_id: "service-tools".into(),
            generation: 1,
            command: vec!["/bin/true".into()],
            timeout_ms: 1_000,
        })
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);

    assert!(matches!(
        client
            .logs(&RuntimeLogQuery {
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
async fn provider_loss_becomes_a_durable_unknown_observation() {
    let driver = Arc::new(TestDriver::new());
    let (client, store) = driver.client();
    let running = client
        .apply(&apply("apply-loss", service("service-loss", 1)))
        .await
        .unwrap();
    driver.missing_on_inspect.store(true, Ordering::SeqCst);

    let inspection = client.inspect("service-loss").await.unwrap();
    let RuntimeInspection::Found { observation } = inspection else {
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
    assert_eq!(record.requests.len(), 32);
    assert!(record
        .requests
        .values()
        .all(|receipt| receipt.state == a3s_runtime::RuntimeRequestState::Pending));
}

struct CountingFactory {
    provider: ProviderId,
    creates: AtomicUsize,
    client: Arc<dyn RuntimeClient>,
}

impl RuntimeProviderFactory for CountingFactory {
    fn provider_id(&self) -> &ProviderId {
        &self.provider
    }

    fn create(&self) -> RuntimeResult<Arc<dyn RuntimeClient>> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(self.client.clone())
    }
}

#[test]
fn provider_registry_never_falls_back_or_replaces_a_factory() {
    let driver = Arc::new(TestDriver::new());
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
        registry.connect(&missing),
        Err(RuntimeError::ProviderUnavailable(_))
    ));
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);
    assert!(registry.register(factory.clone()).is_err());
    assert_eq!(factory.creates.load(Ordering::SeqCst), 0);

    registry.connect(factory.provider_id()).unwrap();
    assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn file_state_store_rejects_symbolic_link_boundaries() {
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
    let lock_path = std::fs::read_dir(root.join("locks"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::remove_file(&lock_path).unwrap();
    let lock_target = root.join("lock-target");
    std::fs::write(&lock_target, b"do not follow").unwrap();
    symlink(&lock_target, &lock_path).unwrap();
    assert!(matches!(
        store.load("secure-unit").await,
        Err(RuntimeError::Protocol(_))
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
