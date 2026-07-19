mod support;

use a3s_runtime::contract::{RuntimeInspection, RuntimeLogQuery};
use a3s_runtime::{
    FileRuntimeStateStore, ManagedRuntimeClient, RuntimeClient, RuntimeError, RuntimeRequestState,
    RuntimeStateStore,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use support::fault_driver::{DeterministicFaultDriver, FaultBoundary, FaultMode, FaultOperation};
use support::fixtures::{action_request, apply_request, exec_request};

fn client(
    directory: &tempfile::TempDir,
    driver: Arc<DeterministicFaultDriver>,
) -> (ManagedRuntimeClient, Arc<FileRuntimeStateStore>) {
    let store = Arc::new(FileRuntimeStateStore::new(directory.path()));
    (ManagedRuntimeClient::new(store.clone(), driver), store)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_millis() as u64
}

fn one_if(condition: bool) -> usize {
    if condition {
        1
    } else {
        0
    }
}

#[tokio::test]
async fn fault_apply_001_transport_boundaries_replay_without_duplicate_resources() {
    for boundary in [FaultBoundary::BeforeEffect, FaultBoundary::AfterEffect] {
        let directory = tempfile::tempdir().expect("fault state root");
        let driver = Arc::new(DeterministicFaultDriver::new());
        let (client, store) = client(&directory, driver.clone());
        let unit_id = format!("fault-apply-{boundary:?}").to_ascii_lowercase();
        let request = apply_request("fault-apply-request", &unit_id);
        driver.arm(FaultOperation::Apply, boundary, FaultMode::Transport);

        assert!(matches!(
            client.apply(&request).await,
            Err(RuntimeError::Transport(message)) if message.contains("injected Apply")
        ));
        assert_eq!(
            driver.resource_count(&unit_id),
            one_if(boundary == FaultBoundary::AfterEffect)
        );
        assert_eq!(
            store
                .load_request(&unit_id, &request.request_id)
                .await
                .expect("pending apply receipt")
                .state,
            RuntimeRequestState::Pending
        );

        let restarted = ManagedRuntimeClient::new(store.clone(), driver.clone());
        let recovered = restarted.apply(&request).await.expect("replay apply");
        let expected_resource_id = format!("provider/{unit_id}/g1");
        assert_eq!(
            recovered.provider_resource_id.as_deref(),
            Some(expected_resource_id.as_str())
        );
        assert_eq!(driver.resource_count(&unit_id), 1);
        assert_eq!(driver.effect_count(FaultOperation::Apply), 1);
        assert_eq!(
            store
                .load_request(&unit_id, &request.request_id)
                .await
                .expect("completed apply receipt")
                .state,
            RuntimeRequestState::Completed
        );
        assert!(driver
            .trace()
            .iter()
            .any(|event| event.operation == FaultOperation::Apply && event.boundary == boundary));
    }
}

#[tokio::test]
async fn fault_mutation_002_stop_remove_and_exec_replay_each_provider_boundary() {
    for boundary in [FaultBoundary::BeforeEffect, FaultBoundary::AfterEffect] {
        let directory = tempfile::tempdir().expect("stop fault state root");
        let driver = Arc::new(DeterministicFaultDriver::new());
        let (client, store) = client(&directory, driver.clone());
        let unit_id = format!("fault-stop-{boundary:?}").to_ascii_lowercase();
        client
            .apply(&apply_request("fault-stop-apply", &unit_id))
            .await
            .expect("prepare running stop fixture");
        let request = action_request("fault-stop-request", &unit_id);
        driver.arm(FaultOperation::Stop, boundary, FaultMode::Transport);

        assert!(matches!(
            client.stop(&request).await,
            Err(RuntimeError::Transport(message)) if message.contains("injected Stop")
        ));
        let expected_after_fault = match boundary {
            FaultBoundary::BeforeEffect => a3s_runtime::contract::RuntimeUnitState::Running,
            FaultBoundary::AfterEffect => a3s_runtime::contract::RuntimeUnitState::Stopped,
        };
        assert_eq!(
            driver.resource_state(&unit_id, 1),
            Some(expected_after_fault)
        );
        let restarted = ManagedRuntimeClient::new(store.clone(), driver.clone());
        let RuntimeInspection::Found { observation, .. } =
            restarted.stop(&request).await.expect("replay stop")
        else {
            panic!("replayed stop must preserve the unit");
        };
        assert_eq!(
            observation.state,
            a3s_runtime::contract::RuntimeUnitState::Stopped
        );
        assert_eq!(driver.effect_count(FaultOperation::Stop), 1);
        assert_eq!(
            store
                .load_request(&unit_id, &request.request_id)
                .await
                .expect("completed stop receipt")
                .state,
            RuntimeRequestState::Completed
        );
    }

    for boundary in [FaultBoundary::BeforeEffect, FaultBoundary::AfterEffect] {
        let directory = tempfile::tempdir().expect("remove fault state root");
        let driver = Arc::new(DeterministicFaultDriver::new());
        let (client, store) = client(&directory, driver.clone());
        let unit_id = format!("fault-remove-{boundary:?}").to_ascii_lowercase();
        client
            .apply(&apply_request("fault-remove-apply", &unit_id))
            .await
            .expect("prepare running remove fixture");
        let request = action_request("fault-remove-request", &unit_id);
        driver.arm(FaultOperation::Remove, boundary, FaultMode::Transport);

        assert!(matches!(
            client.remove(&request).await,
            Err(RuntimeError::Transport(message)) if message.contains("injected Remove")
        ));
        assert_eq!(
            driver.resource_count(&unit_id),
            one_if(boundary == FaultBoundary::BeforeEffect)
        );
        let restarted = ManagedRuntimeClient::new(store.clone(), driver.clone());
        restarted.remove(&request).await.expect("replay removal");
        assert_eq!(driver.resource_count(&unit_id), 0);
        assert_eq!(driver.effect_count(FaultOperation::Remove), 1);
        assert_eq!(
            store
                .load_request(&unit_id, &request.request_id)
                .await
                .expect("completed remove receipt")
                .state,
            RuntimeRequestState::Completed
        );
    }

    for boundary in [FaultBoundary::BeforeEffect, FaultBoundary::AfterEffect] {
        let directory = tempfile::tempdir().expect("exec fault state root");
        let driver = Arc::new(DeterministicFaultDriver::new());
        let (client, store) = client(&directory, driver.clone());
        let unit_id = format!("fault-exec-{boundary:?}").to_ascii_lowercase();
        client
            .apply(&apply_request("fault-exec-apply", &unit_id))
            .await
            .expect("prepare running exec fixture");
        let request = exec_request("fault-exec-request", &unit_id);
        driver.arm(FaultOperation::Exec, boundary, FaultMode::Transport);

        assert!(matches!(
            client.exec(&request).await,
            Err(RuntimeError::Transport(message)) if message.contains("injected Exec")
        ));
        assert_eq!(
            driver.effect_count(FaultOperation::Exec),
            one_if(boundary == FaultBoundary::AfterEffect)
        );
        assert_eq!(
            store
                .load_request(&unit_id, &request.request_id)
                .await
                .expect("pending exec receipt")
                .state,
            RuntimeRequestState::Pending
        );
        let restarted = ManagedRuntimeClient::new(store.clone(), driver.clone());
        let result = restarted.exec(&request).await.expect("replay exec");
        assert_eq!(result.stdout, "ok\n");
        assert_eq!(driver.effect_count(FaultOperation::Exec), 1);
        assert_eq!(
            restarted.exec(&request).await.expect("exact exec replay"),
            result
        );
        assert_eq!(driver.effect_count(FaultOperation::Exec), 1);
    }
}

#[tokio::test]
async fn fault_apply_003_timeout_after_effect_needs_a_new_request_to_reconcile() {
    let directory = tempfile::tempdir().expect("timeout fault state root");
    let driver = Arc::new(DeterministicFaultDriver::new());
    let (client, store) = client(&directory, driver.clone());
    let unit_id = "fault-apply-timeout";
    let mut request = apply_request("fault-timeout-request", unit_id);
    request.deadline_at_ms = Some(now_ms() + 200);
    driver.arm(
        FaultOperation::Apply,
        FaultBoundary::AfterEffect,
        FaultMode::Hang,
    );

    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::DeadlineExceeded(message)) if message.contains("provider apply")
    ));
    assert_eq!(driver.resource_count(unit_id), 1);
    assert_eq!(driver.effect_count(FaultOperation::Apply), 1);
    assert_eq!(
        store
            .load_request(unit_id, &request.request_id)
            .await
            .expect("timed-out pending receipt")
            .state,
        RuntimeRequestState::Pending
    );
    assert!(matches!(
        client.apply(&request).await,
        Err(RuntimeError::DeadlineExceeded(_))
    ));
    assert_eq!(driver.effect_count(FaultOperation::Apply), 1);

    let replacement_request = apply_request("fault-timeout-reconcile", unit_id);
    let recovered = client
        .apply(&replacement_request)
        .await
        .expect("new request reattaches timed-out provider effect");
    assert_eq!(
        recovered.state,
        a3s_runtime::contract::RuntimeUnitState::Running
    );
    assert_eq!(driver.resource_count(unit_id), 1);
    assert_eq!(driver.effect_count(FaultOperation::Apply), 1);
}

#[tokio::test]
async fn fault_apply_004_provider_panic_releases_the_operation_lease() {
    let directory = tempfile::tempdir().expect("panic fault state root");
    let driver = Arc::new(DeterministicFaultDriver::new());
    let (client, store) = client(&directory, driver.clone());
    let client = Arc::new(client);
    let unit_id = "fault-apply-panic";
    let request = apply_request("fault-panic-request", unit_id);
    driver.arm(
        FaultOperation::Apply,
        FaultBoundary::AfterEffect,
        FaultMode::Panic,
    );

    let operation = {
        let client = client.clone();
        let request = request.clone();
        tokio::spawn(async move { client.apply(&request).await })
    };
    assert!(operation
        .await
        .expect_err("provider panic must unwind")
        .is_panic());
    assert_eq!(driver.resource_count(unit_id), 1);
    assert_eq!(
        store
            .load_request(unit_id, &request.request_id)
            .await
            .expect("panic pending receipt")
            .state,
        RuntimeRequestState::Pending
    );

    let restarted = ManagedRuntimeClient::new(store, driver.clone());
    let recovered = tokio::time::timeout(Duration::from_secs(1), restarted.apply(&request))
        .await
        .expect("panic retained operation lease")
        .expect("replay apply after provider panic");
    assert_eq!(
        recovered.state,
        a3s_runtime::contract::RuntimeUnitState::Running
    );
    assert_eq!(driver.effect_count(FaultOperation::Apply), 1);
}

#[tokio::test]
async fn fault_read_005_capabilities_inspect_and_logs_fail_without_state_mutation() {
    for boundary in [FaultBoundary::BeforeEffect, FaultBoundary::AfterEffect] {
        let directory = tempfile::tempdir().expect("capability fault state root");
        let driver = Arc::new(DeterministicFaultDriver::new());
        let (client, store) = client(&directory, driver.clone());
        let unit_id = format!("fault-capability-{boundary:?}").to_ascii_lowercase();
        driver.arm(FaultOperation::Capabilities, boundary, FaultMode::Transport);
        assert!(matches!(
            client.apply(&apply_request("fault-capability-request", &unit_id)).await,
            Err(RuntimeError::Transport(message)) if message.contains("injected Capabilities")
        ));
        assert!(matches!(
            store.load(&unit_id).await,
            Err(RuntimeError::NotFound { .. })
        ));
        assert_eq!(driver.resource_count(&unit_id), 0);
    }

    let directory = tempfile::tempdir().expect("read fault state root");
    let driver = Arc::new(DeterministicFaultDriver::new());
    let (client, store) = client(&directory, driver.clone());
    let unit_id = "fault-read-unit";
    client
        .apply(&apply_request("fault-read-apply", unit_id))
        .await
        .expect("prepare read fault fixture");
    let expected = store.load(unit_id).await.expect("durable read fixture");

    for boundary in [FaultBoundary::BeforeEffect, FaultBoundary::AfterEffect] {
        driver.arm(FaultOperation::Inspect, boundary, FaultMode::Transport);
        assert!(matches!(
            client.inspect(unit_id).await,
            Err(RuntimeError::Transport(message)) if message.contains("injected Inspect")
        ));
        assert_eq!(
            store
                .load(unit_id)
                .await
                .expect("state after inspect fault"),
            expected
        );
    }

    let query = RuntimeLogQuery {
        schema: RuntimeLogQuery::SCHEMA.into(),
        unit_id: unit_id.into(),
        generation: 1,
        cursor: None,
        limit: 10,
        stream: None,
    };
    for boundary in [FaultBoundary::BeforeEffect, FaultBoundary::AfterEffect] {
        driver.arm(FaultOperation::Logs, boundary, FaultMode::Transport);
        assert!(matches!(
            client.logs(&query).await,
            Err(RuntimeError::Transport(message)) if message.contains("injected Logs")
        ));
        assert_eq!(
            store.load(unit_id).await.expect("state after log fault"),
            expected
        );
    }
}
