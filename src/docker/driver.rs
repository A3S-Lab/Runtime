use super::{DockerArtifactResolver, DockerExecutionPlan, DockerOutcome};
use crate::contract::{
    ExecutionState, RuntimeCapabilities, RuntimeExecutionResult, RuntimeExecutionSpec,
};
use crate::{OperationRecord, RuntimeDriver, RuntimeError, RuntimeResult};
use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;

pub struct DockerDriver {
    executable: PathBuf,
    capabilities: RuntimeCapabilities,
    resolver: Arc<dyn DockerArtifactResolver>,
}

impl DockerDriver {
    pub fn new(
        executable: impl Into<PathBuf>,
        capabilities: RuntimeCapabilities,
        resolver: Arc<dyn DockerArtifactResolver>,
    ) -> Self {
        Self {
            executable: executable.into(),
            capabilities,
            resolver,
        }
    }

    async fn ensure_started(
        &self,
        spec: &RuntimeExecutionSpec,
        queued: &RuntimeExecutionResult,
        plan: &DockerExecutionPlan,
    ) -> RuntimeResult<()> {
        plan.validate()?;
        let name = container_name(&queued.execution_id);
        if self.container_matches(&name, queued).await? {
            self.run_command(&["start", &name]).await?;
            return Ok(());
        }
        let mut command = Command::new(&self.executable);
        command.args([
            "create",
            "--name",
            &name,
            "--label",
            &format!("a3s.runtime.operation={}", spec.operation_id),
            "--label",
            &format!("a3s.runtime.execution={}", queued.execution_id),
            "--label",
            &format!("a3s.runtime.spec={}", queued.spec_digest),
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "256",
            "--memory",
            &spec.resources.memory_bytes.to_string(),
            "--cpus",
            &format_cpu(spec.resources.cpu_millis),
            "--tmpfs",
            &format!(
                "/tmp:rw,noexec,nosuid,nodev,size={}",
                spec.resources.scratch_bytes
            ),
        ]);
        if let Some(platform) = &plan.platform {
            command.args(["--platform", platform]);
        }
        for mount in &plan.mounts {
            let mut value = format!(
                "type=bind,src={},dst={}",
                mount.source.display(),
                mount.target
            );
            if mount.read_only {
                value.push_str(",readonly");
            }
            command.args(["--mount", &value]);
        }
        for (key, value) in &plan.environment {
            command.args(["--env", &format!("{key}={value}")]);
        }
        command.arg(&plan.image).args(&plan.argv);
        run_output(command, "create Docker operation").await?;
        self.run_command(&["start", &name]).await?;
        Ok(())
    }

    async fn container_matches(
        &self,
        name: &str,
        result: &RuntimeExecutionResult,
    ) -> RuntimeResult<bool> {
        let output = Command::new(&self.executable)
            .args(["inspect", "--format", "{{json .Config.Labels}}", name])
            .output()
            .await
            .map_err(|error| {
                RuntimeError::ProviderUnavailable(format!("could not run Docker: {error}"))
            })?;
        if !output.status.success() {
            return Ok(false);
        }
        let labels: std::collections::BTreeMap<String, String> =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                RuntimeError::Protocol(format!("Docker returned invalid labels: {error}"))
            })?;
        let matches = labels.get("a3s.runtime.operation") == Some(&result.operation_id)
            && labels.get("a3s.runtime.execution") == Some(&result.execution_id)
            && labels.get("a3s.runtime.spec") == Some(&result.spec_digest);
        if !matches {
            return Err(RuntimeError::Protocol(format!(
                "Docker container {name:?} exists with another Runtime identity"
            )));
        }
        Ok(true)
    }

    async fn inspect_state(&self, name: &str) -> RuntimeResult<DockerState> {
        let output = Command::new(&self.executable)
            .args(["inspect", "--format", "{{json .State}}", name])
            .output()
            .await
            .map_err(|error| {
                RuntimeError::ProviderUnavailable(format!("could not run Docker: {error}"))
            })?;
        if !output.status.success() {
            return Err(RuntimeError::Transport(format!(
                "could not inspect Docker operation {name:?}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        serde_json::from_slice(&output.stdout).map_err(|error| {
            RuntimeError::Protocol(format!("Docker returned invalid state: {error}"))
        })
    }

    async fn run_command(&self, arguments: &[&str]) -> RuntimeResult<()> {
        let mut command = Command::new(&self.executable);
        command.args(arguments);
        run_output(command, "operate Docker container")
            .await
            .map(|_| ())
    }
}

#[async_trait]
impl RuntimeDriver for DockerDriver {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        let output = Command::new(&self.executable)
            .args(["version", "--format", "{{.Server.Version}}"])
            .output()
            .await
            .map_err(|error| {
                RuntimeError::ProviderUnavailable(format!("could not run Docker: {error}"))
            })?;
        if !output.status.success() {
            return Err(RuntimeError::ProviderUnavailable(format!(
                "Docker preflight failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(self.capabilities.clone())
    }

    async fn start(
        &self,
        spec: &RuntimeExecutionSpec,
        queued: &RuntimeExecutionResult,
    ) -> RuntimeResult<RuntimeExecutionResult> {
        let plan = self.resolver.resolve(spec).await?;
        self.ensure_started(spec, queued, &plan).await?;
        let mut running = queued.clone();
        running.state = ExecutionState::Running;
        running.started_at_ms = Some(now_ms()?);
        Ok(running)
    }

    async fn inspect(&self, operation: &OperationRecord) -> RuntimeResult<RuntimeExecutionResult> {
        let state = self
            .inspect_state(&container_name(&operation.result.execution_id))
            .await?;
        if state.running {
            if operation.result.state == ExecutionState::Queued {
                let mut running = operation.result.clone();
                running.state = ExecutionState::Running;
                running.started_at_ms = Some(now_ms()?);
                return Ok(running);
            }
            return Ok(operation.result.clone());
        }
        let started_at_ms = operation
            .result
            .started_at_ms
            .unwrap_or_else(|| now_ms().unwrap_or(0));
        let outcome = DockerOutcome {
            exit_code: state.exit_code,
            started_at_ms,
            finished_at_ms: now_ms()?,
        };
        self.resolver.complete(operation, &outcome).await
    }

    async fn cancel(&self, operation: &OperationRecord) -> RuntimeResult<RuntimeExecutionResult> {
        let name = container_name(&operation.result.execution_id);
        let output = Command::new(&self.executable)
            .args(["stop", "--time", "10", &name])
            .output()
            .await
            .map_err(|error| {
                RuntimeError::ProviderUnavailable(format!("could not run Docker: {error}"))
            })?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            if !detail.contains("No such container") {
                return Err(RuntimeError::Transport(format!(
                    "could not stop Docker operation: {}",
                    detail.trim()
                )));
            }
        }
        self.resolver.cancelled(operation, now_ms()?).await
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerState {
    running: bool,
    exit_code: i64,
}

fn container_name(execution_id: &str) -> String {
    format!(
        "a3s-runtime-{}",
        &format!("{:x}", Sha256::digest(execution_id.as_bytes()))[..32]
    )
}

fn format_cpu(cpu_millis: u64) -> String {
    format!("{}.{:03}", cpu_millis / 1000, cpu_millis % 1000)
}

fn now_ms() -> RuntimeResult<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| RuntimeError::Protocol(format!("system clock precedes epoch: {error}")))?
        .as_millis();
    u64::try_from(millis).map_err(|_| RuntimeError::Protocol("timestamp overflow".into()))
}

async fn run_output(mut command: Command, action: &str) -> RuntimeResult<Vec<u8>> {
    let output = command.output().await.map_err(|error| {
        RuntimeError::ProviderUnavailable(format!("could not {action}: {error}"))
    })?;
    if !output.status.success() {
        return Err(RuntimeError::Transport(format!(
            "could not {action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        ArtifactRef, NetworkPolicy, OutputArtifact, PrivacyClass, ResourceLimits, RuntimeEvidence,
        RuntimeRole, RuntimeUsage, SubmissionPolicy,
    };
    use crate::{A3sRuntimeClient, FileOperationStore, ManagedRuntimeClient};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Mutex;

    #[test]
    fn docker_identity_and_resources_are_deterministic() {
        assert_eq!(container_name("execution-1"), container_name("execution-1"));
        assert_ne!(container_name("execution-1"), container_name("execution-2"));
        assert_eq!(format_cpu(1), "0.001");
        assert_eq!(format_cpu(2500), "2.500");
    }

    struct TestResolver {
        image: String,
        plans: Mutex<HashMap<String, DockerExecutionPlan>>,
    }

    #[async_trait]
    impl DockerArtifactResolver for TestResolver {
        async fn resolve(&self, spec: &RuntimeExecutionSpec) -> RuntimeResult<DockerExecutionPlan> {
            let plan = DockerExecutionPlan {
                image: self.image.clone(),
                platform: None,
                argv: vec!["/bin/sh".into(), "-c".into(), "exit 0".into()],
                mounts: vec![],
                environment: BTreeMap::new(),
            };
            self.plans
                .lock()
                .unwrap()
                .insert(spec.operation_id.clone(), plan.clone());
            Ok(plan)
        }

        async fn complete(
            &self,
            operation: &OperationRecord,
            outcome: &DockerOutcome,
        ) -> RuntimeResult<RuntimeExecutionResult> {
            if outcome.exit_code != 0 {
                return Err(RuntimeError::Protocol(format!(
                    "test container exited with {}",
                    outcome.exit_code
                )));
            }
            let mut result = operation.result.clone();
            result.state = ExecutionState::Succeeded;
            result.started_at_ms = Some(outcome.started_at_ms);
            result.finished_at_ms = Some(outcome.finished_at_ms);
            result.terminal_checkpoint = Some(OutputArtifact {
                artifact: artifact('b'),
                privacy: PrivacyClass::CandidatePrivate,
            });
            result.submission_snapshot = Some(OutputArtifact {
                artifact: artifact('c'),
                privacy: PrivacyClass::TrialSubmission,
            });
            result.usage = Some(RuntimeUsage {
                wall_time_ms: outcome.finished_at_ms.saturating_sub(outcome.started_at_ms),
                cpu_time_ms: 0,
                peak_memory_bytes: 0,
                input_tokens: 0,
                output_tokens: 0,
            });
            result.evidence = Some(RuntimeEvidence {
                semantics_profile_digest: digest('d'),
                provider_build: "docker-e2e-test".into(),
                spec_digest: result.spec_digest.clone(),
                claims: BTreeMap::new(),
            });
            Ok(result)
        }

        async fn cancelled(
            &self,
            _operation: &OperationRecord,
            _finished_at_ms: u64,
        ) -> RuntimeResult<RuntimeExecutionResult> {
            Err(RuntimeError::Protocol("not exercised".into()))
        }
    }

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn artifact(character: char) -> ArtifactRef {
        ArtifactRef {
            digest: digest(character),
            media_type: "application/vnd.a3s.test.v1".into(),
        }
    }

    fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities {
            schema: RuntimeCapabilities::SCHEMA.into(),
            semantics_profile_digest: digest('d'),
            provider_build: "docker-e2e-test".into(),
            immutable_assets: true,
            role_isolation: true,
            protected_mounts: true,
            protected_typed_results: true,
            terminal_checkpoints: true,
            submission_projection: true,
            network_none: true,
            hard_resource_limits: true,
            durable_operations: true,
            cancellation: true,
            usage_evidence: true,
        }
    }

    fn spec(operation_id: &str) -> RuntimeExecutionSpec {
        RuntimeExecutionSpec {
            schema: RuntimeExecutionSpec::SCHEMA.into(),
            operation_id: operation_id.into(),
            role: RuntimeRole::Candidate,
            asset: artifact('a'),
            work_image: artifact('a'),
            protected_mounts: vec![],
            protected_result_schema: None,
            submission_policy: Some(SubmissionPolicy {
                include: vec!["**".into()],
                exclude: vec![],
                max_files: 1,
                max_total_bytes: 1,
                max_file_bytes: 1,
            }),
            network: NetworkPolicy::None,
            resources: ResourceLimits {
                wall_time_ms: 30_000,
                cpu_millis: 1_000,
                memory_bytes: 64 * 1024 * 1024,
                scratch_bytes: 8 * 1024 * 1024,
                output_bytes: 1024 * 1024,
            },
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires a local Docker Engine"]
    async fn managed_docker_candidate_runs_and_reattaches_terminal_result() {
        let image = docker_image_id("alpine:3.20").await;
        let resolver = Arc::new(TestResolver {
            image,
            plans: Mutex::new(HashMap::new()),
        });
        let driver = Arc::new(DockerDriver::new("docker", capabilities(), resolver));
        let directory = tempfile::tempdir().unwrap();
        let client =
            ManagedRuntimeClient::new(Arc::new(FileOperationStore::new(directory.path())), driver);
        let request = spec("runtime-test/docker-candidate");
        let running = client.submit(&request).await.unwrap();
        let name = container_name(&running.execution_id);
        let terminal = loop {
            let result = client.inspect(&request.operation_id).await.unwrap();
            if result.state.is_terminal() {
                break result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert_eq!(terminal.state, ExecutionState::Succeeded);
        terminal.validate().unwrap();
        assert_eq!(
            client.inspect(&request.operation_id).await.unwrap(),
            terminal
        );
        let _ = Command::new("docker")
            .args(["rm", "--force", &name])
            .output()
            .await;
    }

    async fn docker_image_id(reference: &str) -> String {
        let inspect = |reference: &str| {
            let mut command = Command::new("docker");
            command.args(["image", "inspect", "--format", "{{.Id}}", reference]);
            command
        };
        let mut command = inspect(reference);
        let mut output = command.output().await.unwrap();
        if !output.status.success() {
            let pull = Command::new("docker")
                .args(["pull", reference])
                .output()
                .await
                .unwrap();
            assert!(
                pull.status.success(),
                "{}",
                String::from_utf8_lossy(&pull.stderr)
            );
            let mut command = inspect(reference);
            output = command.output().await.unwrap();
        }
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
}
