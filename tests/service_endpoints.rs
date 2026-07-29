use a3s_runtime::contract::{
    ArtifactRef, IsolationLevel, NetworkMode, ResourceControl, ResourceLimits, RestartPolicy,
    RuntimeCapabilities, RuntimeEvidence, RuntimeFeature, RuntimeNetworkSpec, RuntimeObservation,
    RuntimePort, RuntimeProcessSpec, RuntimeServiceEndpoint, RuntimeUnitClass, RuntimeUnitSpec,
    RuntimeUnitState, TransportProtocol,
};
use a3s_runtime::{runtime_profile_requirements, ProviderId, RuntimeConformanceProfile};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};

fn service_spec(protocol: TransportProtocol) -> RuntimeUnitSpec {
    RuntimeUnitSpec {
        schema: RuntimeUnitSpec::SCHEMA.into(),
        unit_id: "service-endpoint-test".into(),
        generation: 3,
        class: RuntimeUnitClass::Service,
        artifact: ArtifactRef {
            uri: format!(
                "oci://registry.example/a3s/service@sha256:{}",
                "a".repeat(64)
            ),
            digest: format!("sha256:{}", "a".repeat(64)),
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
            mode: NetworkMode::Service,
            ports: vec![RuntimePort {
                name: "api".into(),
                container_port: 8080,
                protocol,
            }],
        },
        resources: ResourceLimits {
            cpu_millis: 500,
            memory_bytes: 128 * 1024 * 1024,
            pids: 128,
            ephemeral_storage_bytes: None,
            execution_timeout_ms: None,
        },
        isolation: IsolationLevel::Sandbox,
        health: None,
        restart: RestartPolicy::Always,
        outputs: Vec::new(),
        semantics_profile_digest: None,
    }
}

fn running_observation(spec: &RuntimeUnitSpec) -> RuntimeObservation {
    RuntimeObservation {
        schema: RuntimeObservation::SCHEMA.into(),
        unit_id: spec.unit_id.clone(),
        generation: spec.generation,
        spec_digest: spec.digest().expect("spec digest"),
        class: RuntimeUnitClass::Service,
        state: RuntimeUnitState::Running,
        provider_resource_id: Some("provider/service-endpoint-test".into()),
        provider_build: Some("provider/1".into()),
        observed_at_ms: 20,
        started_at_ms: Some(10),
        finished_at_ms: None,
        health: None,
        outputs: Vec::new(),
        usage: None,
        evidence: Some(RuntimeEvidence {
            provider_build: "provider/1".into(),
            spec_digest: spec.digest().expect("spec digest"),
            semantics_profile_digest: None,
            claims: BTreeMap::new(),
        }),
        provider_attestation: None,
        failure: None,
    }
}

fn capabilities(service_features: Vec<RuntimeFeature>) -> RuntimeCapabilities {
    let mut features = vec![RuntimeFeature::DurableIdentity];
    features.extend(service_features);
    RuntimeCapabilities {
        schema: RuntimeCapabilities::SCHEMA.into(),
        provider_id: ProviderId::parse("endpoint-provider").expect("provider ID"),
        provider_build: "endpoint-provider/1".into(),
        unit_classes: vec![RuntimeUnitClass::Service],
        artifact_media_types: vec!["application/vnd.oci.image.manifest.v1+json".into()],
        isolation_levels: vec![IsolationLevel::Sandbox],
        network_modes: vec![NetworkMode::Service],
        mount_kinds: Vec::new(),
        health_check_kinds: Vec::new(),
        resource_controls: vec![
            ResourceControl::Cpu,
            ResourceControl::Memory,
            ResourceControl::Pids,
        ],
        features,
    }
}

#[test]
fn typed_service_endpoint_is_exact_loopback_evidence() {
    let spec = service_spec(TransportProtocol::Tcp);
    let endpoint = RuntimeServiceEndpoint::new(
        "api",
        TransportProtocol::Tcp,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        49_152,
    )
    .expect("node-local endpoint");
    let mut observation = running_observation(&spec);
    endpoint
        .insert_claim(&mut observation.evidence.as_mut().expect("evidence").claims)
        .expect("endpoint claim");

    observation
        .validate_against(&spec)
        .expect("typed endpoint matches declared Runtime port");
    assert_eq!(
        RuntimeServiceEndpoint::from_observation(&observation, "api").expect("published endpoint"),
        endpoint
    );
    assert_eq!(endpoint.claim_value(), "tcp://127.0.0.1:49152");

    assert!(RuntimeServiceEndpoint::new(
        "api",
        TransportProtocol::Tcp,
        "192.0.2.1".parse().expect("test IP"),
        49_152,
    )
    .is_err());
    assert!(RuntimeServiceEndpoint::node_local_tcp("a".repeat(255), 49_152).is_err());

    let mut missing = running_observation(&spec);
    assert!(missing.validate_against(&spec).is_err());
    missing.state = RuntimeUnitState::Stopped;
    missing.finished_at_ms = Some(21);
    endpoint
        .insert_claim(&mut missing.evidence.as_mut().expect("evidence").claims)
        .expect("endpoint claim");
    assert!(missing.validate_against(&spec).is_err());
    missing
        .evidence
        .as_mut()
        .expect("evidence")
        .claims
        .insert("provider.unrelated".into(), "preserved".into());
    missing.clear_service_endpoints();
    missing
        .validate_against(&spec)
        .expect("terminal observation without endpoint claims");
    assert_eq!(
        missing
            .evidence
            .expect("unrelated evidence")
            .claims
            .get("provider.unrelated")
            .map(String::as_str),
        Some("preserved")
    );
}

#[test]
fn service_protocol_matching_is_capability_exact() {
    let tcp_only = capabilities(vec![RuntimeFeature::ServiceTcp]);
    tcp_only.validate().expect("TCP Service capability");
    assert!(tcp_only
        .missing_for(&service_spec(TransportProtocol::Tcp))
        .expect("TCP capability match")
        .is_empty());
    assert_eq!(
        tcp_only
            .missing_for(&service_spec(TransportProtocol::Udp))
            .expect("UDP capability mismatch"),
        vec!["feature:ServiceUdp"]
    );
    let requirements =
        runtime_profile_requirements(&tcp_only, RuntimeConformanceProfile::Networking)
            .expect("TCP-only Networking requirements");
    assert!(requirements.case_ids.contains("NETWORK-PROTOCOL-TCP"));
    assert!(!requirements.case_ids.contains("NETWORK-PROTOCOL-UDP"));

    assert!(capabilities(Vec::new()).validate().is_err());
    let none_with_tcp = RuntimeCapabilities {
        network_modes: vec![NetworkMode::None],
        ..tcp_only
    };
    assert!(none_with_tcp.validate().is_err());
}
