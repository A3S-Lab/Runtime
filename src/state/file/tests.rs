use super::*;
use crate::contract::{
    ArtifactRef, IsolationLevel, NetworkMode, ResourceLimits, RestartPolicy, RuntimeActionRequest,
    RuntimeApplyRequest, RuntimeExecRequest, RuntimeExecResult, RuntimeNetworkSpec,
    RuntimeObservation, RuntimeProcessSpec, RuntimeRemoval, RuntimeUnitClass, RuntimeUnitSpec,
    RuntimeUnitState,
};
use crate::RuntimeRequestState;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

const NOW: u64 = 10_000;
const UNIT_ID: &str = "receipt-first-unit";
const APPLY_ID: &str = "receipt-first-apply";
const STOP_ID: &str = "receipt-first-stop";
const REMOVE_ID: &str = "receipt-first-remove";
const EXEC_ID: &str = "receipt-first-exec";
const FAILPOINT_ENV: &str = "A3S_RUNTIME_TEST_FAILPOINT";
const READY_ENV: &str = "A3S_RUNTIME_TEST_FAILPOINT_READY";
const ROOT_ENV: &str = "A3S_RUNTIME_TEST_STATE_ROOT";
const OPERATION_ENV: &str = "A3S_RUNTIME_TEST_COMPLETION_KIND";
const OBSERVATION_FAILPOINT: &str = "state.complete-observation.after-receipt-publish";
const REMOVAL_FAILPOINT: &str = "state.complete-removal.after-receipt-publish";
const EXEC_FAILPOINT: &str = "state.complete-exec.after-receipt-publish";

pub(super) fn hit_failpoint(name: &str) {
    if !matches!(std::env::var(FAILPOINT_ENV), Ok(value) if value == name) {
        return;
    }
    let ready = std::env::var(READY_ENV).expect("test failpoint ready path");
    std::fs::write(ready, name).expect("publish test failpoint readiness");
    loop {
        thread::park_timeout(Duration::from_secs(60));
    }
}

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn spec() -> RuntimeUnitSpec {
    RuntimeUnitSpec {
        schema: RuntimeUnitSpec::SCHEMA.into(),
        unit_id: UNIT_ID.into(),
        generation: 1,
        class: RuntimeUnitClass::Service,
        artifact: ArtifactRef {
            uri: format!(
                "oci://registry.example/a3s/runtime@sha256:{}",
                "a".repeat(64)
            ),
            digest: digest('a'),
            media_type: "application/vnd.oci.image.manifest.v1+json".into(),
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

fn apply_request() -> RuntimeApplyRequest {
    RuntimeApplyRequest {
        schema: RuntimeApplyRequest::SCHEMA.into(),
        request_id: APPLY_ID.into(),
        deadline_at_ms: Some(NOW + 60_000),
        spec: spec(),
    }
}

fn action(request_id: &str) -> RuntimeActionRequest {
    RuntimeActionRequest {
        schema: RuntimeActionRequest::SCHEMA.into(),
        request_id: request_id.into(),
        unit_id: UNIT_ID.into(),
        generation: 1,
        deadline_at_ms: Some(NOW + 60_000),
    }
}

fn exec_request() -> RuntimeExecRequest {
    RuntimeExecRequest {
        schema: RuntimeExecRequest::SCHEMA.into(),
        request_id: EXEC_ID.into(),
        unit_id: UNIT_ID.into(),
        generation: 1,
        command: vec!["/bin/true".into()],
        timeout_ms: 1_000,
        deadline_at_ms: Some(NOW + 60_000),
    }
}

fn provider_observation(
    mut observation: RuntimeObservation,
    state: RuntimeUnitState,
    observed_at_ms: u64,
) -> RuntimeObservation {
    observation.state = state;
    observation.provider_resource_id = Some(format!("provider/{UNIT_ID}/1"));
    observation.provider_build = Some("receipt-first-driver/1".into());
    observation.observed_at_ms = observed_at_ms;
    observation.started_at_ms = Some(NOW);
    observation.finished_at_ms = state.is_terminal().then_some(observed_at_ms);
    observation.health = None;
    observation.outputs.clear();
    observation.failure = None;
    observation
}

fn prepare_running(store: &FileRuntimeStateStore) -> RuntimeObservation {
    let request = apply_request();
    let reservation = store
        .reserve_apply_sync(request.clone(), NOW)
        .expect("reserve initial apply");
    let running = provider_observation(
        reservation.record.observation,
        RuntimeUnitState::Running,
        NOW + 1,
    );
    store
        .update_observation_sync(Some(request.request_id), running.clone())
        .expect("complete initial apply");
    running
}

fn spawn_completion_helper(root: &Path, ready: &Path, operation: &str, failpoint: &str) -> Child {
    Command::new(std::env::current_exe().expect("current test executable"))
        .arg("subprocess_receipt_first_completion_helper")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(ROOT_ENV, root)
        .env(READY_ENV, ready)
        .env(OPERATION_ENV, operation)
        .env(FAILPOINT_ENV, failpoint)
        .spawn()
        .expect("spawn receipt-first completion helper")
}

fn kill_at_failpoint(child: &mut Child, ready: &Path, case_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.is_file() {
        if let Some(status) = child.try_wait().expect("inspect completion helper") {
            panic!("{case_id}: helper exited before failpoint: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "{case_id}: helper did not reach failpoint"
        );
        thread::sleep(Duration::from_millis(10));
    }
    child.kill().expect("kill completion helper");
    let status = child.wait().expect("wait for killed completion helper");
    assert!(!status.success(), "{case_id}: helper was not killed");
}

fn run_receipt_first_crash(
    operation: &str,
    failpoint: &str,
    prepare: impl FnOnce(&FileRuntimeStateStore),
    verify: impl FnOnce(&FileRuntimeStateStore),
) {
    let directory = tempfile::tempdir().expect("receipt-first state root");
    let store = FileRuntimeStateStore::new(directory.path());
    prepare(&store);
    let ready = directory.path().join(format!("{operation}.ready"));
    let mut child = spawn_completion_helper(directory.path(), &ready, operation, failpoint);
    kill_at_failpoint(&mut child, &ready, operation);
    verify(&FileRuntimeStateStore::new(directory.path()));
}

#[test]
fn subprocess_receipt_first_completion_helper() {
    let Ok(root) = std::env::var(ROOT_ENV) else {
        return;
    };
    let operation = std::env::var(OPERATION_ENV).expect("completion kind");
    let store = FileRuntimeStateStore::new(PathBuf::from(root));
    let record = store.load_sync(UNIT_ID).expect("load helper unit");
    match operation.as_str() {
        "crash-apply-receipt-first" => {
            let running =
                provider_observation(record.observation, RuntimeUnitState::Running, NOW + 1);
            store
                .update_observation_sync(Some(APPLY_ID.into()), running)
                .expect("complete apply");
        }
        "crash-stop-receipt-first" => {
            let stopped =
                provider_observation(record.observation, RuntimeUnitState::Stopped, NOW + 2);
            store
                .update_observation_sync(Some(STOP_ID.into()), stopped)
                .expect("complete stop");
        }
        "crash-remove-receipt-first" => {
            store
                .complete_removal_sync(RuntimeRemoval {
                    schema: RuntimeRemoval::SCHEMA.into(),
                    request_id: REMOVE_ID.into(),
                    unit_id: UNIT_ID.into(),
                    generation: 1,
                    removed_at_ms: NOW + 3,
                    already_absent: false,
                })
                .expect("complete removal");
        }
        "crash-exec-receipt-first" => {
            let mut refreshed = record.observation;
            refreshed.observed_at_ms += 1;
            store
                .complete_exec_sync(RuntimeExecResult {
                    schema: RuntimeExecResult::SCHEMA.into(),
                    request_id: EXEC_ID.into(),
                    observation: refreshed,
                    exit_code: 0,
                    stdout: "ok\n".into(),
                    stderr: String::new(),
                    truncated: false,
                })
                .expect("complete exec");
        }
        other => panic!("unknown receipt-first completion kind {other:?}"),
    }
}

#[test]
fn state_crash_001_receipt_first_process_kills_reconcile_every_result_kind() {
    run_receipt_first_crash(
        "crash-apply-receipt-first",
        OBSERVATION_FAILPOINT,
        |store| {
            store
                .reserve_apply_sync(apply_request(), NOW)
                .expect("reserve apply");
        },
        |store| {
            assert_eq!(
                store
                    .load_sync(UNIT_ID)
                    .expect("pre-replay unit")
                    .observation
                    .state,
                RuntimeUnitState::Accepted
            );
            assert_eq!(
                store
                    .load_request_sync(UNIT_ID, APPLY_ID)
                    .expect("completed apply receipt")
                    .state,
                RuntimeRequestState::Completed
            );
            let replay = store
                .reserve_apply_sync(apply_request(), NOW + 2)
                .expect("replay apply");
            assert!(!replay.dispatch);
            assert_eq!(replay.record.observation.state, RuntimeUnitState::Running);
            assert_eq!(
                store
                    .load_sync(UNIT_ID)
                    .expect("reconciled unit")
                    .observation,
                replay.record.observation
            );
        },
    );

    run_receipt_first_crash(
        "crash-stop-receipt-first",
        OBSERVATION_FAILPOINT,
        |store| {
            prepare_running(store);
            store
                .reserve_action_sync(RuntimeActionKind::Stop, action(STOP_ID), NOW + 1)
                .expect("reserve stop");
        },
        |store| {
            assert_eq!(
                store
                    .load_sync(UNIT_ID)
                    .expect("pre-replay unit")
                    .observation
                    .state,
                RuntimeUnitState::Running
            );
            let replay = store
                .reserve_action_sync(RuntimeActionKind::Stop, action(STOP_ID), NOW + 3)
                .expect("replay stop");
            assert!(!replay.dispatch);
            assert_eq!(replay.record.observation.state, RuntimeUnitState::Stopped);
        },
    );

    run_receipt_first_crash(
        "crash-remove-receipt-first",
        REMOVAL_FAILPOINT,
        |store| {
            prepare_running(store);
            store
                .reserve_action_sync(RuntimeActionKind::Remove, action(REMOVE_ID), NOW + 1)
                .expect("reserve removal");
        },
        |store| {
            assert!(store
                .load_sync(UNIT_ID)
                .expect("pre-replay unit")
                .removed_at_ms
                .is_none());
            let replay = store
                .reserve_action_sync(RuntimeActionKind::Remove, action(REMOVE_ID), NOW + 4)
                .expect("replay removal");
            assert!(!replay.dispatch);
            assert_eq!(replay.record.removed_at_ms, Some(NOW + 3));
        },
    );

    run_receipt_first_crash(
        "crash-exec-receipt-first",
        EXEC_FAILPOINT,
        |store| {
            prepare_running(store);
            store
                .reserve_exec_sync(exec_request(), NOW + 1)
                .expect("reserve exec");
        },
        |store| {
            let before = store
                .load_sync(UNIT_ID)
                .expect("pre-replay unit")
                .observation
                .observed_at_ms;
            let replay = store
                .reserve_exec_sync(exec_request(), NOW + 2)
                .expect("replay exec");
            assert!(!replay.dispatch);
            assert!(replay.record.observation.observed_at_ms > before);
            assert_eq!(
                replay.receipt.state,
                RuntimeRequestState::Completed,
                "exec result must remain durable after process kill"
            );
        },
    );
}
