use super::{
    ArtifactRef, IsolationLevel, ResourceLimits, RuntimeNetworkSpec, RuntimeProcessSpec,
    SecretReference,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUnitClass {
    Task,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountKind {
    Artifact,
    Volume,
    Tmpfs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeMountSource {
    Artifact { artifact: ArtifactRef },
    Volume { volume_id: String },
    Tmpfs { size_bytes: u64 },
}

impl RuntimeMountSource {
    pub fn kind(&self) -> MountKind {
        match self {
            Self::Artifact { .. } => MountKind::Artifact,
            Self::Volume { .. } => MountKind::Volume,
            Self::Tmpfs { .. } => MountKind::Tmpfs,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Artifact { artifact } => artifact.validate(),
            Self::Volume { volume_id } => super::validate_id("volume_id", volume_id, 255),
            Self::Tmpfs { size_bytes } if *size_bytes == 0 => {
                Err("tmpfs size_bytes must be positive".into())
            }
            Self::Tmpfs { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeMount {
    pub name: String,
    pub source: RuntimeMountSource,
    pub target: String,
    pub read_only: bool,
}

impl RuntimeMount {
    fn validate(&self) -> Result<(), String> {
        super::validate_name("mount name", &self.name)?;
        super::validate_absolute_path("mount target", &self.target)?;
        self.source.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckKind {
    Http,
    Tcp,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HealthProbe {
    Http {
        port: String,
        path: String,
        expected_statuses: Vec<u16>,
    },
    Tcp {
        port: String,
    },
    Command {
        command: Vec<String>,
    },
}

impl HealthProbe {
    pub fn kind(&self) -> HealthCheckKind {
        match self {
            Self::Http { .. } => HealthCheckKind::Http,
            Self::Tcp { .. } => HealthCheckKind::Tcp,
            Self::Command { .. } => HealthCheckKind::Command,
        }
    }

    fn validate(&self, network: &RuntimeNetworkSpec) -> Result<(), String> {
        match self {
            Self::Http {
                port,
                path,
                expected_statuses,
            } => {
                super::validate_name("health port", port)?;
                if !network.has_port(port) {
                    return Err(format!(
                        "HTTP health check references unknown port {port:?}"
                    ));
                }
                if !path.starts_with('/') || path.len() > 2048 || path.contains(['\0', '\r', '\n'])
                {
                    return Err("HTTP health path must be a bounded absolute request path".into());
                }
                if expected_statuses.is_empty()
                    || expected_statuses.len() > 32
                    || expected_statuses
                        .iter()
                        .any(|status| !(100..=599).contains(status))
                {
                    return Err("HTTP health expected_statuses are invalid".into());
                }
                Ok(())
            }
            Self::Tcp { port } => {
                super::validate_name("health port", port)?;
                if !network.has_port(port) {
                    return Err(format!("TCP health check references unknown port {port:?}"));
                }
                Ok(())
            }
            Self::Command { command } => {
                if command.is_empty() || command.len() > 64 {
                    return Err("command health check requires 1 to 64 arguments".into());
                }
                for value in command {
                    if value.is_empty() || value.len() > 32 * 1024 || value.contains('\0') {
                        return Err("command health check contains an invalid argument".into());
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHealthCheck {
    pub probe: HealthProbe,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub start_period_ms: u64,
    pub success_threshold: u32,
    pub failure_threshold: u32,
}

impl RuntimeHealthCheck {
    fn validate(&self, network: &RuntimeNetworkSpec) -> Result<(), String> {
        if self.interval_ms == 0
            || self.timeout_ms == 0
            || self.timeout_ms > self.interval_ms
            || self.success_threshold == 0
            || self.failure_threshold == 0
        {
            return Err("health timing and threshold values are invalid".into());
        }
        self.probe.validate(network)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RestartPolicy {
    Never,
    OnFailure { max_retries: u32 },
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOutputSpec {
    pub name: String,
    pub path: String,
    pub media_type: String,
    pub max_bytes: u64,
}

impl RuntimeOutputSpec {
    fn validate(&self) -> Result<(), String> {
        super::validate_name("output name", &self.name)?;
        super::validate_absolute_path("output path", &self.path)?;
        super::validate_nonempty("output media_type", &self.media_type, 255)?;
        if self.max_bytes == 0 {
            return Err("output max_bytes must be positive".into());
        }
        Ok(())
    }
}

/// Immutable provider-neutral definition of one finite Task or long-running
/// Service generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeUnitSpec {
    pub schema: String,
    pub unit_id: String,
    pub generation: u64,
    pub class: RuntimeUnitClass,
    pub artifact: ArtifactRef,
    pub process: RuntimeProcessSpec,
    pub mounts: Vec<RuntimeMount>,
    pub secrets: Vec<SecretReference>,
    pub network: RuntimeNetworkSpec,
    pub resources: ResourceLimits,
    pub isolation: IsolationLevel,
    pub health: Option<RuntimeHealthCheck>,
    pub restart: RestartPolicy,
    pub outputs: Vec<RuntimeOutputSpec>,
    pub semantics_profile_digest: Option<String>,
}

impl RuntimeUnitSpec {
    pub const SCHEMA: &'static str = "a3s.runtime.unit-spec.v2";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!("unsupported Runtime unit schema {:?}", self.schema));
        }
        super::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0 {
            return Err("Runtime unit generation must be positive".into());
        }
        self.artifact.validate()?;
        self.process.validate()?;
        self.network.validate()?;
        self.resources.validate()?;
        if self.mounts.len() > 128 || self.secrets.len() > 128 || self.outputs.len() > 128 {
            return Err("Runtime unit input or output count exceeds protocol limits".into());
        }

        let mut mount_names = BTreeSet::new();
        let mut mount_targets = BTreeSet::new();
        for mount in &self.mounts {
            mount.validate()?;
            if !mount_names.insert(&mount.name) || !mount_targets.insert(&mount.target) {
                return Err("Runtime mount names and targets must be unique".into());
            }
        }

        let mut secret_names = BTreeSet::new();
        let mut secret_targets = BTreeSet::new();
        for secret in &self.secrets {
            secret.validate()?;
            let target = serde_json::to_string(&secret.target)
                .map_err(|error| format!("could not encode secret target: {error}"))?;
            if !secret_names.insert(&secret.name) || !secret_targets.insert(target) {
                return Err("Runtime secret names and targets must be unique".into());
            }
        }

        let mut output_names = BTreeSet::new();
        let mut output_paths = BTreeSet::new();
        for output in &self.outputs {
            output.validate()?;
            if !output_names.insert(&output.name) || !output_paths.insert(&output.path) {
                return Err("Runtime output names and paths must be unique".into());
            }
        }

        if let Some(digest) = &self.semantics_profile_digest {
            super::validate_digest(digest)?;
        }

        match self.class {
            RuntimeUnitClass::Task => {
                if self.resources.execution_timeout_ms.is_none() {
                    return Err("Task requires execution_timeout_ms".into());
                }
                if self.health.is_some() || matches!(self.restart, RestartPolicy::Always) {
                    return Err("Task cannot use health checks or an always restart policy".into());
                }
            }
            RuntimeUnitClass::Service => {
                if self.resources.execution_timeout_ms.is_some() || !self.outputs.is_empty() {
                    return Err("Service cannot use an execution timeout or Task outputs".into());
                }
                if let Some(health) = &self.health {
                    health.validate(&self.network)?;
                }
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| format!("could not encode Runtime unit spec: {error}"))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn artifact() -> ArtifactRef {
        ArtifactRef {
            uri: format!("oci://registry.example/a3s/demo@sha256:{}", "a".repeat(64)),
            digest: format!("sha256:{}", "a".repeat(64)),
            media_type: "application/vnd.oci.image.manifest.v1+json".into(),
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

    fn task() -> RuntimeUnitSpec {
        RuntimeUnitSpec {
            schema: RuntimeUnitSpec::SCHEMA.into(),
            unit_id: "build-1".into(),
            generation: 1,
            class: RuntimeUnitClass::Task,
            artifact: artifact(),
            process: RuntimeProcessSpec {
                command: vec!["/bin/build".into()],
                args: vec![],
                working_directory: Some("/workspace".into()),
                environment: BTreeMap::new(),
            },
            mounts: vec![],
            secrets: vec![],
            network: RuntimeNetworkSpec {
                mode: super::super::NetworkMode::Outbound,
                ports: vec![],
            },
            resources: resources(Some(60_000)),
            isolation: IsolationLevel::Container,
            health: None,
            restart: RestartPolicy::OnFailure { max_retries: 1 },
            outputs: vec![RuntimeOutputSpec {
                name: "image".into(),
                path: "/outputs/image.json".into(),
                media_type: "application/json".into(),
                max_bytes: 1024,
            }],
            semantics_profile_digest: None,
        }
    }

    fn service() -> RuntimeUnitSpec {
        let mut spec = task();
        spec.unit_id = "service-1".into();
        spec.class = RuntimeUnitClass::Service;
        spec.resources = resources(None);
        spec.outputs.clear();
        spec.restart = RestartPolicy::Always;
        spec.network = RuntimeNetworkSpec {
            mode: super::super::NetworkMode::Service,
            ports: vec![super::super::RuntimePort {
                name: "http".into(),
                container_port: 8080,
                protocol: super::super::TransportProtocol::Tcp,
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
            start_period_ms: 10_000,
            success_threshold: 1,
            failure_threshold: 3,
        });
        spec
    }

    #[test]
    fn task_and_service_specs_are_general_and_digest_stable() {
        let task = task();
        let service = service();
        task.validate().unwrap();
        service.validate().unwrap();
        assert_eq!(task.digest().unwrap(), task.digest().unwrap());
        assert_ne!(task.digest().unwrap(), service.digest().unwrap());
    }

    #[test]
    fn lifecycle_specific_fields_fail_closed() {
        let mut task = task();
        task.resources.execution_timeout_ms = None;
        assert!(task.validate().is_err());

        let mut service = service();
        service.outputs.push(RuntimeOutputSpec {
            name: "invalid".into(),
            path: "/output".into(),
            media_type: "text/plain".into(),
            max_bytes: 1,
        });
        assert!(service.validate().is_err());
    }

    #[test]
    fn health_checks_reference_declared_ports() {
        let mut service = service();
        let health = service.health.as_mut().unwrap();
        health.probe = HealthProbe::Tcp {
            port: "missing".into(),
        };
        assert!(service.validate().is_err());
    }
}
