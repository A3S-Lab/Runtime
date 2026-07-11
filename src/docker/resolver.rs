use super::DockerExecutionPlan;
use crate::contract::{RuntimeExecutionResult, RuntimeExecutionSpec};
use crate::{OperationRecord, RuntimeResult};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerOutcome {
    pub exit_code: i64,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
}

/// Resolves immutable Runtime artifacts into a provider-local container plan
/// and finalizes provider output into protected Runtime artifacts.
#[async_trait]
pub trait DockerArtifactResolver: Send + Sync {
    async fn resolve(&self, spec: &RuntimeExecutionSpec) -> RuntimeResult<DockerExecutionPlan>;

    async fn complete(
        &self,
        operation: &OperationRecord,
        outcome: &DockerOutcome,
    ) -> RuntimeResult<RuntimeExecutionResult>;

    async fn cancelled(
        &self,
        operation: &OperationRecord,
        finished_at_ms: u64,
    ) -> RuntimeResult<RuntimeExecutionResult>;
}
