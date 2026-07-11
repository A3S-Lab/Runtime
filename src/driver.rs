use crate::contract::{RuntimeCapabilities, RuntimeExecutionResult, RuntimeExecutionSpec};
use crate::{OperationRecord, RuntimeResult};
use async_trait::async_trait;

/// Provider-specific execution primitive used by `ManagedRuntimeClient`.
///
/// Drivers do not own idempotency or persistence. They receive the durable
/// record identity chosen by the shared client and return the next complete
/// protocol result for that same identity.
#[async_trait]
pub trait RuntimeDriver: Send + Sync {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities>;

    async fn start(
        &self,
        spec: &RuntimeExecutionSpec,
        queued: &RuntimeExecutionResult,
    ) -> RuntimeResult<RuntimeExecutionResult>;

    async fn inspect(&self, operation: &OperationRecord) -> RuntimeResult<RuntimeExecutionResult>;

    async fn cancel(&self, operation: &OperationRecord) -> RuntimeResult<RuntimeExecutionResult>;
}
