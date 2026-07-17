use super::file::validate_transition;
use crate::contract::{
    ArtifactRef, IsolationLevel, NetworkMode, ResourceLimits, RestartPolicy, RuntimeFailure,
    RuntimeHealthObservation, RuntimeHealthState, RuntimeNetworkSpec, RuntimeObservation,
    RuntimeProcessSpec, RuntimeUnitClass, RuntimeUnitSpec, RuntimeUnitState,
};
use std::collections::BTreeMap;

const STATES: [RuntimeUnitState; 9] = [
    RuntimeUnitState::Accepted,
    RuntimeUnitState::Preparing,
    RuntimeUnitState::Starting,
    RuntimeUnitState::Running,
    RuntimeUnitState::Stopping,
    RuntimeUnitState::Stopped,
    RuntimeUnitState::Succeeded,
    RuntimeUnitState::Failed,
    RuntimeUnitState::Unknown,
];

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn spec(class: RuntimeUnitClass) -> RuntimeUnitSpec {
    let class_name = match class {
        RuntimeUnitClass::Task => "task",
        RuntimeUnitClass::Service => "service",
    };
    RuntimeUnitSpec {
        schema: RuntimeUnitSpec::SCHEMA.into(),
        unit_id: format!("transition-{class_name}"),
        generation: 1,
        class,
        artifact: ArtifactRef {
            uri: format!(
                "oci://registry.example/a3s/transition@sha256:{}",
                "a".repeat(64)
            ),
            digest: digest('a'),
            media_type: "application/vnd.oci.image.manifest.v1+json".into(),
        },
        process: RuntimeProcessSpec {
            command: vec!["/bin/workload".into()],
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
            execution_timeout_ms: (class == RuntimeUnitClass::Task).then_some(60_000),
        },
        isolation: IsolationLevel::Container,
        health: None,
        restart: match class {
            RuntimeUnitClass::Task => RestartPolicy::Never,
            RuntimeUnitClass::Service => RestartPolicy::Always,
        },
        outputs: Vec::new(),
        semantics_profile_digest: None,
    }
}

fn observation(
    spec: &RuntimeUnitSpec,
    state: RuntimeUnitState,
    observed_at_ms: u64,
) -> RuntimeObservation {
    let provider_backed = state != RuntimeUnitState::Accepted;
    RuntimeObservation {
        schema: RuntimeObservation::SCHEMA.into(),
        unit_id: spec.unit_id.clone(),
        generation: spec.generation,
        spec_digest: spec.digest().expect("valid transition specification"),
        class: spec.class,
        state,
        provider_resource_id: provider_backed.then(|| "provider/transition/1".into()),
        provider_build: provider_backed.then(|| "transition-driver/1".into()),
        observed_at_ms,
        started_at_ms: provider_backed.then_some(100),
        finished_at_ms: state.is_terminal().then_some(observed_at_ms),
        health: None,
        outputs: Vec::new(),
        usage: None,
        evidence: None,
        provider_attestation: None,
        failure: (state == RuntimeUnitState::Failed).then(|| RuntimeFailure {
            code: "provider_failed".into(),
            message: "injected transition failure".into(),
            retryable: false,
        }),
    }
}

fn valid_for_class(class: RuntimeUnitClass, state: RuntimeUnitState) -> bool {
    class != RuntimeUnitClass::Service || state != RuntimeUnitState::Succeeded
}

fn expected_transition(from: RuntimeUnitState, to: RuntimeUnitState) -> bool {
    if from.is_terminal() {
        return false;
    }
    from == to
        || matches!(
            (from, to),
            (RuntimeUnitState::Accepted, RuntimeUnitState::Preparing)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Starting)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Running)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Starting)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Running)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Running)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Running, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Running, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Running, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Running, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Running, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Stopping, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Stopping, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Stopping, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Preparing)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Starting)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Running)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Failed)
        )
}

#[test]
fn state_transition_001_task_and_service_matrices_have_complete_oracles() {
    for class in [RuntimeUnitClass::Task, RuntimeUnitClass::Service] {
        let spec = spec(class);
        for from in STATES
            .into_iter()
            .filter(|state| valid_for_class(class, *state))
        {
            for to in STATES
                .into_iter()
                .filter(|state| valid_for_class(class, *state))
            {
                let current = observation(&spec, from, 200);
                let next = observation(&spec, to, 201);
                let actual = validate_transition(&current, &next, &spec).is_ok();
                assert_eq!(
                    actual,
                    expected_transition(from, to),
                    "STATE-TRANSITION-{class:?}-{from:?}-{to:?}"
                );
            }
        }
    }
}

#[test]
fn state_transition_002_terminal_identity_time_and_class_rules_fail_closed() {
    for class in [RuntimeUnitClass::Task, RuntimeUnitClass::Service] {
        let spec = spec(class);
        for terminal in [
            RuntimeUnitState::Stopped,
            RuntimeUnitState::Succeeded,
            RuntimeUnitState::Failed,
        ]
        .into_iter()
        .filter(|state| valid_for_class(class, *state))
        {
            let current = observation(&spec, terminal, 200);
            assert!(
                validate_transition(&current, &current, &spec).is_ok(),
                "STATE-TERMINAL-REPLAY-{class:?}-{terminal:?}"
            );
        }
    }

    let task = spec(RuntimeUnitClass::Task);
    let running = observation(&task, RuntimeUnitState::Running, 200);
    let mut substituted = observation(&task, RuntimeUnitState::Running, 201);
    substituted.provider_resource_id = Some("provider/substituted/1".into());
    assert!(validate_transition(&running, &substituted, &task).is_err());

    let unknown = observation(&task, RuntimeUnitState::Unknown, 200);
    assert!(validate_transition(&unknown, &substituted, &task).is_ok());
    let accepted = observation(&task, RuntimeUnitState::Accepted, 201);
    assert!(validate_transition(&unknown, &accepted, &task).is_err());

    let mut backwards = observation(&task, RuntimeUnitState::Running, 199);
    backwards.provider_resource_id = running.provider_resource_id.clone();
    assert!(validate_transition(&running, &backwards, &task).is_err());

    let service = spec(RuntimeUnitClass::Service);
    let service_running = observation(&service, RuntimeUnitState::Running, 200);
    let service_succeeded = observation(&service, RuntimeUnitState::Succeeded, 201);
    assert!(validate_transition(&service_running, &service_succeeded, &service).is_err());

    let mut task_with_health = observation(&task, RuntimeUnitState::Running, 201);
    task_with_health.health = Some(RuntimeHealthObservation {
        state: RuntimeHealthState::Healthy,
        checked_at_ms: 201,
        message: None,
    });
    assert!(validate_transition(&running, &task_with_health, &task).is_err());
}
