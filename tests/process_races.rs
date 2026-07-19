#[path = "process_races/driver.rs"]
mod driver;

use a3s_runtime::contract::{
    ArtifactRef, IsolationLevel, NetworkMode, ResourceLimits, RestartPolicy, RuntimeActionRequest,
    RuntimeApplyRequest, RuntimeNetworkSpec, RuntimeProcessSpec, RuntimeUnitClass, RuntimeUnitSpec,
    RuntimeUnitState,
};
use a3s_runtime::{
    FileRuntimeStateStore, ManagedRuntimeClient, RuntimeClient, RuntimeError, RuntimeRequestState,
    RuntimeStateStore,
};
use driver::{ProcessRaceDriver, ProviderResource, IMAGE_MEDIA_TYPE};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const STATE_ROOT_ENV: &str = "A3S_RUNTIME_PROCESS_STATE_ROOT";
const PROVIDER_ROOT_ENV: &str = "A3S_RUNTIME_PROCESS_PROVIDER_ROOT";
const OPERATION_ENV: &str = "A3S_RUNTIME_PROCESS_OPERATION";
const REQUEST_ID_ENV: &str = "A3S_RUNTIME_PROCESS_REQUEST_ID";
const UNIT_ID_ENV: &str = "A3S_RUNTIME_PROCESS_UNIT_ID";
const GENERATION_ENV: &str = "A3S_RUNTIME_PROCESS_GENERATION";
const START_GATE_ENV: &str = "A3S_RUNTIME_PROCESS_START_GATE";
const READY_ENV: &str = "A3S_RUNTIME_PROCESS_READY";
const RESULT_ENV: &str = "A3S_RUNTIME_PROCESS_RESULT";
const DRIVER_FAILPOINT_ENV: &str = "A3S_RUNTIME_PROCESS_DRIVER_FAILPOINT";
const DRIVER_FAILPOINT_READY_ENV: &str = "A3S_RUNTIME_PROCESS_DRIVER_FAILPOINT_READY";
const APPLY_AFTER_PUBLISH: &str = "provider.apply.after-current-publish";

fn service_spec(unit_id: &str, generation: u64) -> RuntimeUnitSpec {
    RuntimeUnitSpec {
        schema: RuntimeUnitSpec::SCHEMA.into(),
        unit_id: unit_id.into(),
        generation,
        class: RuntimeUnitClass::Service,
        artifact: ArtifactRef {
            uri: format!(
                "oci://registry.example/a3s/process-race@sha256:{}",
                "a".repeat(64)
            ),
            digest: format!("sha256:{}", "a".repeat(64)),
            media_type: IMAGE_MEDIA_TYPE.into(),
        },
        process: RuntimeProcessSpec {
            command: vec!["/bin/service".into()],
            args: Vec::new(),
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
            execution_timeout_ms: None,
        },
        isolation: IsolationLevel::Container,
        health: None,
        restart: RestartPolicy::Always,
        outputs: Vec::new(),
        semantics_profile_digest: None,
    }
}

fn apply_request(request_id: &str, unit_id: &str, generation: u64) -> RuntimeApplyRequest {
    RuntimeApplyRequest {
        schema: RuntimeApplyRequest::SCHEMA.into(),
        request_id: request_id.into(),
        deadline_at_ms: None,
        spec: service_spec(unit_id, generation),
    }
}

fn action_request(request_id: &str, unit_id: &str, generation: u64) -> RuntimeActionRequest {
    RuntimeActionRequest {
        schema: RuntimeActionRequest::SCHEMA.into(),
        request_id: request_id.into(),
        unit_id: unit_id.into(),
        generation,
        deadline_at_ms: None,
    }
}

fn runtime_client(state_root: &Path, provider_root: &Path) -> ManagedRuntimeClient {
    ManagedRuntimeClient::new(
        Arc::new(FileRuntimeStateStore::new(state_root)),
        Arc::new(ProcessRaceDriver::new(provider_root)),
    )
}

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build process-race test runtime")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subprocess_process_race_operation_helper() {
    let Ok(state_root) = std::env::var(STATE_ROOT_ENV) else {
        return;
    };
    let provider_root = PathBuf::from(std::env::var(PROVIDER_ROOT_ENV).expect("provider root"));
    let operation = std::env::var(OPERATION_ENV).expect("operation");
    let request_id = std::env::var(REQUEST_ID_ENV).expect("request ID");
    let unit_id = std::env::var(UNIT_ID_ENV).expect("unit ID");
    let generation = std::env::var(GENERATION_ENV)
        .expect("generation")
        .parse::<u64>()
        .expect("numeric generation");
    let result = PathBuf::from(std::env::var(RESULT_ENV).expect("result path"));

    if let Ok(ready) = std::env::var(READY_ENV) {
        std::fs::write(ready, b"ready").expect("publish process helper readiness");
    }
    if let Ok(gate) = std::env::var(START_GATE_ENV) {
        let gate = PathBuf::from(gate);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !gate.is_file() {
            assert!(
                Instant::now() < deadline,
                "process helper start gate timed out"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    let client = runtime_client(Path::new(&state_root), &provider_root);
    let outcome = match operation.as_str() {
        "apply" => client
            .apply(&apply_request(&request_id, &unit_id, generation))
            .await
            .map(|_| ()),
        "stop" => client
            .stop(&action_request(&request_id, &unit_id, generation))
            .await
            .map(|_| ()),
        "remove" => client
            .remove(&action_request(&request_id, &unit_id, generation))
            .await
            .map(|_| ()),
        other => panic!("unknown process helper operation {other:?}"),
    };
    std::fs::write(result, classify(outcome)).expect("write process helper result");
}

fn classify(result: Result<(), RuntimeError>) -> &'static str {
    match result {
        Ok(()) => "ok",
        Err(RuntimeError::StaleGeneration { .. }) => "stale-generation",
        Err(RuntimeError::GenerationConflict { .. }) => "generation-conflict",
        Err(RuntimeError::NotFound { .. }) => "not-found",
        Err(RuntimeError::Protocol(_)) => "protocol",
        Err(_) => "unexpected-error",
    }
}

struct ProcessCase {
    child: Child,
    ready: PathBuf,
    result: PathBuf,
}

fn spawn_operation(
    state_root: &Path,
    provider_root: &Path,
    gate: &Path,
    operation: &str,
    request_id: &str,
    unit_id: &str,
    generation: u64,
) -> ProcessCase {
    let result = state_root.join(format!("{request_id}.result"));
    let ready = state_root.join(format!("{request_id}.ready"));
    let child = Command::new(std::env::current_exe().expect("current test executable"))
        .arg("subprocess_process_race_operation_helper")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(STATE_ROOT_ENV, state_root)
        .env(PROVIDER_ROOT_ENV, provider_root)
        .env(OPERATION_ENV, operation)
        .env(REQUEST_ID_ENV, request_id)
        .env(UNIT_ID_ENV, unit_id)
        .env(GENERATION_ENV, generation.to_string())
        .env(START_GATE_ENV, gate)
        .env(READY_ENV, &ready)
        .env(RESULT_ENV, &result)
        .spawn()
        .expect("spawn process race operation");
    ProcessCase {
        child,
        ready,
        result,
    }
}

fn wait_for_ready(case: &mut ProcessCase, case_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !case.ready.is_file() {
        if let Some(status) = case.child.try_wait().expect("inspect process helper") {
            panic!("{case_id}: helper exited before ready: {status}");
        }
        assert!(Instant::now() < deadline, "{case_id}: readiness timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_exit(case: &mut ProcessCase, case_id: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = case.child.try_wait().expect("inspect process helper") {
            assert!(status.success(), "{case_id}: helper failed: {status}");
            return std::fs::read_to_string(&case.result).expect("read process helper result");
        }
        if Instant::now() >= deadline {
            case.child.kill().expect("kill timed-out process helper");
            case.child
                .wait()
                .expect("wait for timed-out process helper");
            panic!("{case_id}: helper timed out");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn run_pair(
    state_root: &Path,
    provider_root: &Path,
    case_id: &str,
    left: (&str, &str, &str, u64),
    right: (&str, &str, &str, u64),
) -> (String, String) {
    let gate = state_root.join(format!("{case_id}.gate"));
    let mut left_case = spawn_operation(
        state_root,
        provider_root,
        &gate,
        left.0,
        left.1,
        left.2,
        left.3,
    );
    let mut right_case = spawn_operation(
        state_root,
        provider_root,
        &gate,
        right.0,
        right.1,
        right.2,
        right.3,
    );
    wait_for_ready(&mut left_case, case_id);
    wait_for_ready(&mut right_case, case_id);
    std::fs::write(&gate, b"start").expect("release process race gate");
    (
        wait_for_exit(&mut left_case, case_id),
        wait_for_exit(&mut right_case, case_id),
    )
}

fn assert_single_generation(resources: &[ProviderResource], generation: u64) {
    assert_eq!(resources.len(), 1, "provider inventory is not singular");
    assert_eq!(resources[0].generation, generation);
}

fn inject_duplicate(provider_root: &Path, unit_id: &str, generation: u64) {
    let driver = ProcessRaceDriver::new(provider_root);
    let source = driver
        .inventory(unit_id)
        .expect("load duplicate source")
        .into_iter()
        .find(|resource| resource.generation == generation)
        .expect("duplicate source is absent");
    let mut duplicate = source;
    duplicate.resource_id = format!("{}/duplicate", duplicate.resource_id);
    let key = format!("{:x}", Sha256::digest(duplicate.resource_id.as_bytes()));
    let path = provider_root.join("resources").join(format!("{key}.json"));
    std::fs::write(
        path,
        serde_json::to_vec(&duplicate).expect("encode duplicate provider resource"),
    )
    .expect("inject duplicate provider resource");
}

#[test]
fn race_gen_001_concurrent_generations_converge_to_one_newest_resource() {
    let state = tempfile::tempdir().expect("generation-race state root");
    let provider = tempfile::tempdir().expect("generation-race provider root");
    let unit_id = "race-generation-unit";
    let (generation_one, generation_two) = run_pair(
        state.path(),
        provider.path(),
        "RACE-GEN-001",
        ("apply", "race-generation-one", unit_id, 1),
        ("apply", "race-generation-two", unit_id, 2),
    );
    assert!(matches!(generation_one.as_str(), "ok" | "stale-generation"));
    assert_eq!(generation_two, "ok");

    let runtime = test_runtime();
    let store = FileRuntimeStateStore::new(state.path());
    let record = runtime
        .block_on(store.load(unit_id))
        .expect("load final unit");
    assert_eq!(record.spec.generation, 2);
    assert_eq!(record.observation.state, RuntimeUnitState::Running);
    assert_single_generation(
        &ProcessRaceDriver::new(provider.path())
            .inventory(unit_id)
            .expect("final provider inventory"),
        2,
    );
}

#[test]
fn race_ops_001_apply_stop_remove_process_races_have_deterministic_oracles() {
    for (suffix, competing_operation) in [("stop", "stop"), ("remove", "remove")] {
        let state = tempfile::tempdir().expect("operation-race state root");
        let provider = tempfile::tempdir().expect("operation-race provider root");
        let unit_id = format!("race-apply-{suffix}");
        let runtime = test_runtime();
        runtime
            .block_on(
                runtime_client(state.path(), provider.path()).apply(&apply_request(
                    &format!("prepare-{suffix}"),
                    &unit_id,
                    1,
                )),
            )
            .expect("prepare operation race");
        let (apply_result, action_result) = run_pair(
            state.path(),
            provider.path(),
            &format!("RACE-OPS-001-apply-{suffix}"),
            ("apply", &format!("apply-{suffix}-g2"), &unit_id, 2),
            (competing_operation, &format!("{suffix}-g1"), &unit_id, 1),
        );
        assert_eq!(apply_result, "ok");
        assert!(matches!(
            action_result.as_str(),
            "ok" | "stale-generation" | "not-found"
        ));
        let record = runtime
            .block_on(FileRuntimeStateStore::new(state.path()).load(&unit_id))
            .expect("load apply/action race unit");
        assert_eq!(record.spec.generation, 2);
        assert_eq!(record.observation.state, RuntimeUnitState::Running);
        assert_single_generation(
            &ProcessRaceDriver::new(provider.path())
                .inventory(&unit_id)
                .expect("apply/action provider inventory"),
            2,
        );
    }

    let state = tempfile::tempdir().expect("stop-remove state root");
    let provider = tempfile::tempdir().expect("stop-remove provider root");
    let unit_id = "race-stop-remove";
    let runtime = test_runtime();
    runtime
        .block_on(
            runtime_client(state.path(), provider.path()).apply(&apply_request(
                "prepare-stop-remove",
                unit_id,
                1,
            )),
        )
        .expect("prepare stop/remove race");
    let (stop_result, remove_result) = run_pair(
        state.path(),
        provider.path(),
        "RACE-OPS-001-stop-remove",
        ("stop", "race-stop", unit_id, 1),
        ("remove", "race-remove", unit_id, 1),
    );
    assert!(matches!(stop_result.as_str(), "ok" | "not-found"));
    assert_eq!(remove_result, "ok");
    let record = runtime
        .block_on(FileRuntimeStateStore::new(state.path()).load(unit_id))
        .expect("load stop/remove race unit");
    assert!(record.removed_at_ms.is_some());
    assert!(ProcessRaceDriver::new(provider.path())
        .inventory(unit_id)
        .expect("stop/remove provider inventory")
        .is_empty());
}

#[test]
fn crash_gen_001_killed_generation_handoff_exact_replay_converges_once() {
    let state = tempfile::tempdir().expect("generation-crash state root");
    let provider = tempfile::tempdir().expect("generation-crash provider root");
    let unit_id = "generation-crash-unit";
    let runtime = test_runtime();
    runtime
        .block_on(
            runtime_client(state.path(), provider.path()).apply(&apply_request(
                "generation-crash-one",
                unit_id,
                1,
            )),
        )
        .expect("prepare generation crash");

    let result = state.path().join("generation-crash-two.result");
    let failpoint_ready = state.path().join("generation-crash-two.failpoint");
    let mut child = Command::new(std::env::current_exe().expect("current test executable"))
        .arg("subprocess_process_race_operation_helper")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(STATE_ROOT_ENV, state.path())
        .env(PROVIDER_ROOT_ENV, provider.path())
        .env(OPERATION_ENV, "apply")
        .env(REQUEST_ID_ENV, "generation-crash-two")
        .env(UNIT_ID_ENV, unit_id)
        .env(GENERATION_ENV, "2")
        .env(RESULT_ENV, &result)
        .env(DRIVER_FAILPOINT_ENV, APPLY_AFTER_PUBLISH)
        .env(DRIVER_FAILPOINT_READY_ENV, &failpoint_ready)
        .spawn()
        .expect("spawn generation crash helper");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !failpoint_ready.is_file() {
        if let Some(status) = child.try_wait().expect("inspect generation crash helper") {
            panic!("generation crash helper exited early: {status}");
        }
        assert!(Instant::now() < deadline, "generation failpoint timed out");
        thread::sleep(Duration::from_millis(10));
    }
    child.kill().expect("kill generation crash helper");
    assert!(!child
        .wait()
        .expect("wait generation crash helper")
        .success());

    let store = FileRuntimeStateStore::new(state.path());
    let interrupted = runtime
        .block_on(store.load(unit_id))
        .expect("load interrupted unit");
    assert_eq!(interrupted.spec.generation, 2);
    assert_eq!(interrupted.observation.state, RuntimeUnitState::Accepted);
    assert_eq!(
        runtime
            .block_on(store.load_request(unit_id, "generation-crash-two"))
            .expect("load interrupted receipt")
            .state,
        RuntimeRequestState::Pending
    );
    let process_driver = ProcessRaceDriver::new(provider.path());
    let interrupted_inventory = process_driver
        .inventory(unit_id)
        .expect("interrupted provider inventory");
    assert_eq!(
        interrupted_inventory
            .iter()
            .map(|resource| resource.generation)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    let request = apply_request("generation-crash-two", unit_id, 2);
    let recovered = runtime
        .block_on(runtime_client(state.path(), provider.path()).apply(&request))
        .expect("recover generation handoff");
    assert_eq!(recovered.state, RuntimeUnitState::Running);
    let converged = process_driver
        .inventory(unit_id)
        .expect("converged provider inventory");
    assert_single_generation(&converged, 2);
    assert_eq!(converged[0].apply_dispatches, 2);
    let replayed = runtime
        .block_on(runtime_client(state.path(), provider.path()).apply(&request))
        .expect("replay recovered generation");
    assert_eq!(replayed, recovered);
    assert_eq!(
        process_driver
            .inventory(unit_id)
            .expect("replayed provider inventory"),
        converged
    );
}

#[test]
fn race_inventory_001_duplicate_resources_fail_closed_before_provider_mutation() {
    let state = tempfile::tempdir().expect("duplicate state root");
    let provider = tempfile::tempdir().expect("duplicate provider root");
    let unit_id = "duplicate-provider-unit";
    let runtime = test_runtime();
    let client = runtime_client(state.path(), provider.path());
    runtime
        .block_on(client.apply(&apply_request("duplicate-one", unit_id, 1)))
        .expect("prepare duplicate provider fixture");
    let process_driver = ProcessRaceDriver::new(provider.path());
    inject_duplicate(provider.path(), unit_id, 1);
    assert_eq!(
        process_driver
            .inventory(unit_id)
            .expect("duplicate provider inventory")
            .len(),
        2
    );
    assert!(matches!(
        runtime.block_on(client.inspect(unit_id)),
        Err(RuntimeError::Protocol(message)) if message.contains("duplicate resources")
    ));
    assert!(matches!(
        runtime.block_on(client.apply(&apply_request("duplicate-two", unit_id, 2))),
        Err(RuntimeError::Protocol(message)) if message.contains("duplicate resources")
    ));
    let resources = process_driver
        .inventory(unit_id)
        .expect("post-rejection provider inventory");
    assert_eq!(resources.len(), 2);
    assert!(resources.iter().all(|resource| resource.generation == 1));
    let record = runtime
        .block_on(FileRuntimeStateStore::new(state.path()).load(unit_id))
        .expect("load duplicate-rejection state");
    assert_eq!(record.spec.generation, 2);
    assert_eq!(record.observation.state, RuntimeUnitState::Accepted);
}
