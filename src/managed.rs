use crate::contract::{RuntimeCapabilities, RuntimeExecutionResult, RuntimeExecutionSpec};
use crate::{A3sRuntimeClient, OperationStore, RuntimeDriver, RuntimeResult};
use async_trait::async_trait;
use std::sync::Arc;

/// Shared durable lifecycle implementation used by concrete providers.
pub struct ManagedRuntimeClient {
    operations: Arc<dyn OperationStore>,
    driver: Arc<dyn RuntimeDriver>,
}

impl ManagedRuntimeClient {
    pub fn new(operations: Arc<dyn OperationStore>, driver: Arc<dyn RuntimeDriver>) -> Self {
        Self { operations, driver }
    }
}

#[async_trait]
impl A3sRuntimeClient for ManagedRuntimeClient {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        let capabilities = self.driver.capabilities().await?;
        capabilities
            .validate()
            .map_err(crate::RuntimeError::Protocol)?;
        Ok(capabilities)
    }

    async fn submit(&self, spec: &RuntimeExecutionSpec) -> RuntimeResult<RuntimeExecutionResult> {
        let reservation = self.operations.reserve(spec).await?;
        if !reservation.created {
            return Ok(reservation.record.result);
        }
        let result = self.driver.start(spec, &reservation.record.result).await?;
        Ok(self.operations.update(&result).await?.result)
    }

    async fn inspect(&self, operation_id: &str) -> RuntimeResult<RuntimeExecutionResult> {
        let record = self.operations.load(operation_id).await?;
        if record.result.state.is_terminal() {
            return Ok(record.result);
        }
        let result = self.driver.inspect(&record).await?;
        Ok(self.operations.update(&result).await?.result)
    }

    async fn cancel(&self, operation_id: &str) -> RuntimeResult<RuntimeExecutionResult> {
        let record = self.operations.load(operation_id).await?;
        if record.result.state.is_terminal() {
            return Ok(record.result);
        }
        let result = self.driver.cancel(&record).await?;
        Ok(self.operations.update(&result).await?.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        ArtifactRef, ExecutionFailure, ExecutionState, NetworkPolicy, ResourceLimits,
        RuntimeEvidence, RuntimeRole, RuntimeUsage, SubmissionPolicy,
    };
    use crate::{FileOperationStore, OperationRecord, RuntimeError};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDriver {
        starts: AtomicUsize,
        inspections: AtomicUsize,
        cancellations: AtomicUsize,
    }

    impl TestDriver {
        fn new() -> Self {
            Self {
                starts: AtomicUsize::new(0),
                inspections: AtomicUsize::new(0),
                cancellations: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl RuntimeDriver for TestDriver {
        async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
            Ok(RuntimeCapabilities {
                schema: RuntimeCapabilities::SCHEMA.into(),
                semantics_profile_digest: digest('c'),
                provider_build: "test".into(),
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
            })
        }

        async fn start(
            &self,
            _spec: &RuntimeExecutionSpec,
            queued: &RuntimeExecutionResult,
        ) -> RuntimeResult<RuntimeExecutionResult> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let mut running = queued.clone();
            running.state = ExecutionState::Running;
            running.started_at_ms = Some(1);
            Ok(running)
        }

        async fn inspect(
            &self,
            operation: &OperationRecord,
        ) -> RuntimeResult<RuntimeExecutionResult> {
            self.inspections.fetch_add(1, Ordering::SeqCst);
            Ok(operation.result.clone())
        }

        async fn cancel(
            &self,
            operation: &OperationRecord,
        ) -> RuntimeResult<RuntimeExecutionResult> {
            self.cancellations.fetch_add(1, Ordering::SeqCst);
            let mut cancelled = operation.result.clone();
            cancelled.state = ExecutionState::Cancelled;
            cancelled.finished_at_ms = Some(2);
            cancelled.failure = Some(ExecutionFailure {
                code: "cancelled".into(),
                message: "cancelled by caller".into(),
                retryable: false,
            });
            cancelled.usage = Some(RuntimeUsage {
                wall_time_ms: 1,
                cpu_time_ms: 0,
                peak_memory_bytes: 0,
                input_tokens: 0,
                output_tokens: 0,
            });
            cancelled.evidence = Some(RuntimeEvidence {
                semantics_profile_digest: digest('c'),
                provider_build: "test".into(),
                spec_digest: operation.result.spec_digest.clone(),
                claims: BTreeMap::new(),
            });
            Ok(cancelled)
        }
    }

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn spec(operation_id: &str) -> RuntimeExecutionSpec {
        let artifact = ArtifactRef {
            digest: digest('a'),
            media_type: "application/vnd.a3s.asset.v1".into(),
        };
        RuntimeExecutionSpec {
            schema: RuntimeExecutionSpec::SCHEMA.into(),
            operation_id: operation_id.into(),
            role: RuntimeRole::Candidate,
            asset: artifact.clone(),
            work_image: artifact,
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
                wall_time_ms: 1,
                cpu_millis: 1,
                memory_bytes: 1,
                scratch_bytes: 1,
                output_bytes: 1,
            },
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn repeated_submit_starts_once_and_cancel_is_durable() {
        let directory = tempfile::tempdir().unwrap();
        let driver = Arc::new(TestDriver::new());
        let client = ManagedRuntimeClient::new(
            Arc::new(FileOperationStore::new(directory.path())),
            driver.clone(),
        );
        let first = client.submit(&spec("run/managed")).await.unwrap();
        assert_eq!(first.state, ExecutionState::Running);
        let repeated = client.submit(&spec("run/managed")).await.unwrap();
        assert_eq!(repeated, first);
        assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
        let cancelled = client.cancel("run/managed").await.unwrap();
        assert_eq!(cancelled.state, ExecutionState::Cancelled);
        assert_eq!(driver.cancellations.load(Ordering::SeqCst), 1);
        assert_eq!(client.inspect("run/managed").await.unwrap(), cancelled);
        assert_eq!(driver.inspections.load(Ordering::SeqCst), 0);
        assert_eq!(client.cancel("run/managed").await.unwrap(), cancelled);
        assert_eq!(driver.cancellations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn driver_identity_substitution_is_rejected_by_store() {
        struct BadDriver;
        #[async_trait]
        impl RuntimeDriver for BadDriver {
            async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
                Err(RuntimeError::Protocol("unused".into()))
            }
            async fn start(
                &self,
                _spec: &RuntimeExecutionSpec,
                queued: &RuntimeExecutionResult,
            ) -> RuntimeResult<RuntimeExecutionResult> {
                let mut result = queued.clone();
                result.state = ExecutionState::Running;
                result.started_at_ms = Some(1);
                result.execution_id = "substituted".into();
                Ok(result)
            }
            async fn inspect(
                &self,
                operation: &OperationRecord,
            ) -> RuntimeResult<RuntimeExecutionResult> {
                Ok(operation.result.clone())
            }
            async fn cancel(
                &self,
                operation: &OperationRecord,
            ) -> RuntimeResult<RuntimeExecutionResult> {
                Ok(operation.result.clone())
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let client = ManagedRuntimeClient::new(
            Arc::new(FileOperationStore::new(directory.path())),
            Arc::new(BadDriver),
        );
        assert!(client.submit(&spec("run/bad-driver")).await.is_err());
    }
}
