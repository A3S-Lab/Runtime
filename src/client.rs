use crate::contract::{RuntimeCapabilities, RuntimeExecutionResult, RuntimeExecutionSpec};
use crate::RuntimeResult;
use async_trait::async_trait;

/// Stable control-plane entry point implemented by every Runtime provider.
///
/// `submit` is idempotent by `operation_id`: the same request reattaches to the
/// existing logical operation, while different request bytes for an existing
/// ID must return `RuntimeError::OperationConflict`.
#[async_trait]
pub trait A3sRuntimeClient: Send + Sync {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities>;

    async fn submit(&self, spec: &RuntimeExecutionSpec) -> RuntimeResult<RuntimeExecutionResult>;

    async fn inspect(&self, operation_id: &str) -> RuntimeResult<RuntimeExecutionResult>;

    async fn cancel(&self, operation_id: &str) -> RuntimeResult<RuntimeExecutionResult>;
}
