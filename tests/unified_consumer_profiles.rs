use a3s_runtime::contract::{
    ArtifactRef, HealthCheckKind, IsolationLevel, MountKind, NetworkMode, ResourceControl,
    RuntimeCapabilities, RuntimeEvidence, RuntimeFeature, RuntimeHealthObservation,
    RuntimeHealthState, RuntimeObservation, RuntimeServiceEndpoint, RuntimeUnitClass,
    RuntimeUnitSpec, RuntimeUnitState,
};
use a3s_runtime::{
    ProviderId, RuntimeAttestationBinding, RuntimeConsumerRequirements, RuntimeError,
};
use std::collections::BTreeMap;
use std::path::Path;

fn spec(fixture: &str) -> RuntimeUnitSpec {
    let source = match fixture {
        "agent" => include_str!("fixtures/agent-service-runtime-unit-spec.json"),
        "function_task" => include_str!("fixtures/function-task-runtime-unit-spec.json"),
        "function_service" => include_str!("fixtures/function-service-runtime-unit-spec.json"),
        "durable_cell" => include_str!("fixtures/durable-cell-service-runtime-unit-spec.json"),
        "mcp" => include_str!("fixtures/mcp0.1-runtime-unit-spec.json"),
        _ => panic!("unknown fixture"),
    };
    serde_json::from_str(source).expect("consumer Runtime fixture")
}

fn capabilities() -> RuntimeCapabilities {
    RuntimeCapabilities {
        schema: RuntimeCapabilities::SCHEMA.into(),
        provider_id: ProviderId::parse("consumer-fixture").expect("provider"),
        provider_build: "consumer-fixture/test".into(),
        unit_classes: vec![RuntimeUnitClass::Task, RuntimeUnitClass::Service],
        artifact_media_types: vec!["application/vnd.oci.image.manifest.v1+json".into()],
        isolation_levels: vec![IsolationLevel::Sandbox],
        network_modes: vec![NetworkMode::Outbound, NetworkMode::Service],
        mount_kinds: vec![MountKind::Volume, MountKind::Tmpfs],
        health_check_kinds: vec![HealthCheckKind::Http],
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
            RuntimeFeature::ServiceTcp,
            RuntimeFeature::Logs,
            RuntimeFeature::Exec,
            RuntimeFeature::SecretReferences,
            RuntimeFeature::OutputArtifacts,
        ],
    }
}

fn running_observation(spec: &RuntimeUnitSpec, port_name: &str) -> RuntimeObservation {
    let mut claims = BTreeMap::new();
    RuntimeServiceEndpoint::node_local_tcp(port_name, 49_152)
        .expect("endpoint")
        .insert_claim(&mut claims)
        .expect("endpoint evidence");
    RuntimeObservation {
        schema: RuntimeObservation::SCHEMA.into(),
        unit_id: spec.unit_id.clone(),
        generation: spec.generation,
        spec_digest: spec.digest().expect("spec digest"),
        class: RuntimeUnitClass::Service,
        state: RuntimeUnitState::Running,
        provider_resource_id: Some(format!("box/{}/{}", spec.unit_id, spec.generation)),
        provider_build: Some("consumer-fixture/test".into()),
        observed_at_ms: 20_000,
        started_at_ms: Some(10_000),
        finished_at_ms: None,
        health: Some(RuntimeHealthObservation {
            state: RuntimeHealthState::Healthy,
            checked_at_ms: 20_000,
            message: None,
        }),
        outputs: Vec::new(),
        usage: None,
        evidence: Some(RuntimeEvidence {
            provider_build: "consumer-fixture/test".into(),
            spec_digest: spec.digest().expect("spec digest"),
            semantics_profile_digest: spec.semantics_profile_digest.clone(),
            identity_attachment_digest: spec.identity_attachment_digest.clone(),
            claims,
        }),
        provider_attestation: None,
        failure: None,
    }
}

fn stateful_service_requirements() -> RuntimeConsumerRequirements {
    RuntimeConsumerRequirements::new(RuntimeUnitClass::Service)
        .require_semantics_profile()
        .require_health()
        .require_service_endpoints()
        .require_feature(RuntimeFeature::ServiceTcp)
        .require_feature(RuntimeFeature::SecretReferences)
        .require_feature(RuntimeFeature::Logs)
        .require_feature(RuntimeFeature::Exec)
}

fn stateless_service_requirements() -> RuntimeConsumerRequirements {
    RuntimeConsumerRequirements::new(RuntimeUnitClass::Service)
        .require_semantics_profile()
        .require_health()
        .require_service_endpoints()
        .require_feature(RuntimeFeature::ServiceTcp)
}

#[test]
fn rcon_agent_001_admits_one_generic_stateful_service_profile() {
    let spec = spec("agent");
    let requirements = stateful_service_requirements();
    requirements
        .admit_spec(&spec, &capabilities())
        .expect("Agent consumer profile");
    requirements
        .accept_observation(&spec, &running_observation(&spec, "harness"))
        .expect("exact Agent Service observation");
}

#[test]
fn rcon_function_001_admits_task_and_stateless_service_without_a_new_unit_class() {
    let task = spec("function_task");
    RuntimeConsumerRequirements::new(RuntimeUnitClass::Task)
        .require_semantics_profile()
        .require_feature(RuntimeFeature::OutputArtifacts)
        .admit_spec(&task, &capabilities())
        .expect("finite Function Task profile");

    let service = spec("function_service");
    let requirements = stateless_service_requirements();
    requirements
        .admit_spec(&service, &capabilities())
        .expect("stateless Function Service profile");
    requirements
        .accept_observation(&service, &running_observation(&service, "http"))
        .expect("exact Function Service observation");
}

#[test]
fn rcon_mcp_001_reuses_the_same_stateless_service_requirements() {
    let mcp = spec("mcp");
    let requirements = stateless_service_requirements();
    requirements
        .admit_spec(&mcp, &capabilities())
        .expect("sessionless MCP Service profile");
    requirements
        .accept_observation(&mcp, &running_observation(&mcp, "mcp"))
        .expect("exact MCP Service observation");
}

#[test]
fn rcon_cell_001_admits_named_state_without_a_new_unit_class() {
    let cell = spec("durable_cell");
    let requirements = RuntimeConsumerRequirements::new(RuntimeUnitClass::Service)
        .require_semantics_profile()
        .require_health()
        .require_service_endpoints()
        .require_feature(RuntimeFeature::ServiceTcp)
        .require_feature(RuntimeFeature::SecretReferences)
        .require_feature(RuntimeFeature::Logs);
    requirements
        .admit_spec(&cell, &capabilities())
        .expect("named state Service profile");
    requirements
        .accept_observation(&cell, &running_observation(&cell, "cell"))
        .expect("exact named state Service observation");
}

#[test]
fn rcon_fence_001_fails_closed_on_missing_profile_capability_or_evidence() {
    let mut agent = spec("agent");
    agent.semantics_profile_digest = None;
    assert!(matches!(
        stateful_service_requirements().admit_spec(&agent, &capabilities()),
        Err(RuntimeError::InvalidRequest(_))
    ));

    let agent = spec("agent");
    let mut insufficient = capabilities();
    insufficient
        .features
        .retain(|feature| *feature != RuntimeFeature::Exec);
    assert!(matches!(
        stateful_service_requirements().admit_spec(&agent, &insufficient),
        Err(RuntimeError::UnsupportedCapabilities(missing))
            if missing == vec!["feature:Exec"]
    ));

    let mut observation = running_observation(&agent, "harness");
    observation.evidence = None;
    assert!(matches!(
        stateful_service_requirements().accept_observation(&agent, &observation),
        Err(RuntimeError::Protocol(_))
    ));
}

#[test]
fn rcon_identity_001_binds_one_opaque_attachment_to_exact_attested_evidence() {
    let mut agent = spec("agent");
    agent.identity_attachment_digest = Some(format!("sha256:{}", "9".repeat(64)));
    let requirements = stateful_service_requirements().require_identity_attestation();

    let mut provider = capabilities();
    provider.features.extend([
        RuntimeFeature::Attestation,
        RuntimeFeature::IdentityAttachment,
    ]);
    requirements
        .admit_spec(&agent, &provider)
        .expect("identity-attached specification");

    let mut observation = running_observation(&agent, "harness");
    observation.provider_attestation = Some(ArtifactRef {
        uri: format!(
            "attestation://consumer-fixture/{}/{}@sha256:{}",
            agent.unit_id,
            agent.generation,
            "8".repeat(64)
        ),
        digest: format!("sha256:{}", "8".repeat(64)),
        media_type: "application/vnd.a3s.runtime-attestation.v1+json".into(),
    });
    requirements
        .accept_observation(&agent, &observation)
        .expect("exact identity-attached attestation");
    let binding = RuntimeAttestationBinding::from_observation(&agent, &observation)
        .expect("typed attestation binding");
    assert_eq!(binding.unit_id, agent.unit_id);
    assert_eq!(binding.generation, agent.generation);
    assert_eq!(binding.digest().unwrap(), binding.digest().unwrap());

    let mut drifted = observation.clone();
    drifted
        .evidence
        .as_mut()
        .expect("evidence")
        .identity_attachment_digest = Some(format!("sha256:{}", "7".repeat(64)));
    assert!(matches!(
        requirements.accept_observation(&agent, &drifted),
        Err(RuntimeError::Protocol(_))
    ));

    let mut build_drifted = observation.clone();
    build_drifted
        .evidence
        .as_mut()
        .expect("evidence")
        .provider_build = "consumer-fixture/other".into();
    assert!(matches!(
        requirements.accept_observation(&agent, &build_drifted),
        Err(RuntimeError::Protocol(_))
    ));

    let mut unattested = observation;
    unattested.provider_attestation = None;
    assert!(matches!(
        requirements.accept_observation(&agent, &unattested),
        Err(RuntimeError::Protocol(_))
    ));
}

#[test]
fn rcon_boundary_001_runtime_wire_stays_product_neutral() {
    let contract = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/contract");
    let mut violations = Vec::new();
    for entry in std::fs::read_dir(contract).expect("contract directory") {
        let entry = entry.expect("contract entry");
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let lower = source.to_ascii_lowercase();
        for forbidden in ["agent", "function", "workflow", "mcp", "durable cell"] {
            if lower.contains(forbidden) {
                violations.push(format!("{} contains {forbidden:?}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "product semantics entered the Runtime wire contract:\n{}",
        violations.join("\n")
    );
}
