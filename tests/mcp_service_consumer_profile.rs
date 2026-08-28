use a3s_runtime::contract::{
    NetworkMode, RuntimeEvidence, RuntimeHealthObservation, RuntimeHealthState, RuntimeObservation,
    RuntimeServiceEndpoint, RuntimeUnitClass, RuntimeUnitSpec, RuntimeUnitState, TransportProtocol,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const PROFILE_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn fixture_spec() -> RuntimeUnitSpec {
    serde_json::from_str(include_str!("fixtures/mcp0.1-runtime-unit-spec.json"))
        .expect("MCP0.1 Runtime fixture must decode")
}

fn running_observation(spec: &RuntimeUnitSpec) -> RuntimeObservation {
    let mut claims = BTreeMap::new();
    RuntimeServiceEndpoint::node_local_tcp("mcp", 49_152)
        .expect("typed loopback endpoint")
        .insert_claim(&mut claims)
        .expect("endpoint evidence");
    RuntimeObservation {
        schema: RuntimeObservation::SCHEMA.into(),
        unit_id: spec.unit_id.clone(),
        generation: spec.generation,
        spec_digest: spec.digest().expect("fixture spec digest"),
        class: RuntimeUnitClass::Service,
        state: RuntimeUnitState::Running,
        provider_resource_id: Some("box/mcp-weather/replica-1/generation-3".into()),
        provider_build: Some("a3s-box/mcp0.1-fixture".into()),
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
            provider_build: "a3s-box/mcp0.1-fixture".into(),
            spec_digest: spec.digest().expect("fixture spec digest"),
            semantics_profile_digest: spec.semantics_profile_digest.clone(),
            claims,
        }),
        provider_attestation: None,
        failure: None,
    }
}

#[test]
fn rmcp_fix_001_uses_only_the_generic_runtime_service_contract() {
    // Git may materialize the fixture with CRLF on Windows. The locked
    // cross-repository identity is the repository's LF byte form.
    let fixture = include_str!("fixtures/mcp0.1-runtime-unit-spec.json").replace("\r\n", "\n");
    assert_eq!(
        format!("{:x}", Sha256::digest(fixture.as_bytes())),
        "5915c0ccac040fc4270ee5095de58b9115caee6e240464863cd6c3c1dcd59d23"
    );
    let spec = fixture_spec();
    spec.validate().expect("valid Runtime Service fixture");

    assert_eq!(spec.class, RuntimeUnitClass::Service);
    assert_eq!(spec.network.mode, NetworkMode::Service);
    assert_eq!(spec.network.ports.len(), 1);
    assert_eq!(spec.network.ports[0].name, "mcp");
    assert_eq!(spec.network.ports[0].protocol, TransportProtocol::Tcp);
    assert_eq!(
        spec.semantics_profile_digest.as_deref(),
        Some(PROFILE_DIGEST)
    );

    let encoded = serde_json::to_value(&spec).expect("fixture re-encode");
    let source: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mcp0.1-runtime-unit-spec.json"))
            .expect("fixture JSON");
    assert_eq!(encoded, source);
}

#[test]
fn rmcp_endpoint_001_binds_unit_generation_profile_and_typed_endpoint() {
    let spec = fixture_spec();
    let observation = running_observation(&spec);
    observation
        .validate_against(&spec)
        .expect("exact Runtime observation");

    let endpoint =
        RuntimeServiceEndpoint::from_observation(&observation, "mcp").expect("MCP endpoint");
    assert_eq!(endpoint.claim_value(), "tcp://127.0.0.1:49152");
    assert_eq!(
        observation
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.semantics_profile_digest.as_deref()),
        Some(PROFILE_DIGEST)
    );
}

#[test]
fn rmcp_fence_001_rejects_stale_generation_and_profile_evidence() {
    let spec = fixture_spec();

    let mut stale = running_observation(&spec);
    stale.generation -= 1;
    assert!(stale.validate_against(&spec).is_err());

    let mut mixed = running_observation(&spec);
    mixed
        .evidence
        .as_mut()
        .expect("fixture evidence")
        .semantics_profile_digest =
        Some("sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
    let error = mixed
        .validate_against(&spec)
        .expect_err("profile mismatch must fail");
    assert!(error.contains("semantics profile"));
}
