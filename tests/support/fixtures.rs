use super::fault_driver::IMAGE_MEDIA_TYPE;
use a3s_runtime::contract::{
    ArtifactRef, IsolationLevel, NetworkMode, ResourceLimits, RestartPolicy, RuntimeActionRequest,
    RuntimeApplyRequest, RuntimeExecRequest, RuntimeNetworkSpec, RuntimeProcessSpec,
    RuntimeUnitClass, RuntimeUnitSpec,
};
use std::collections::BTreeMap;

pub fn service_spec(unit_id: &str) -> RuntimeUnitSpec {
    RuntimeUnitSpec {
        schema: RuntimeUnitSpec::SCHEMA.into(),
        unit_id: unit_id.into(),
        generation: 1,
        class: RuntimeUnitClass::Service,
        artifact: ArtifactRef {
            uri: format!("oci://registry.example/a3s/fault@sha256:{}", "a".repeat(64)),
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

pub fn apply_request(request_id: &str, unit_id: &str) -> RuntimeApplyRequest {
    RuntimeApplyRequest {
        schema: RuntimeApplyRequest::SCHEMA.into(),
        request_id: request_id.into(),
        deadline_at_ms: None,
        spec: service_spec(unit_id),
    }
}

pub fn action_request(request_id: &str, unit_id: &str) -> RuntimeActionRequest {
    RuntimeActionRequest {
        schema: RuntimeActionRequest::SCHEMA.into(),
        request_id: request_id.into(),
        unit_id: unit_id.into(),
        generation: 1,
        deadline_at_ms: None,
    }
}

pub fn exec_request(request_id: &str, unit_id: &str) -> RuntimeExecRequest {
    RuntimeExecRequest {
        schema: RuntimeExecRequest::SCHEMA.into(),
        request_id: request_id.into(),
        unit_id: unit_id.into(),
        generation: 1,
        command: vec!["/bin/true".into()],
        timeout_ms: 1_000,
        deadline_at_ms: None,
    }
}
