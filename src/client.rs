use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeCapabilities, RuntimeExecRequest,
    RuntimeExecResult, RuntimeInspection, RuntimeLogChunk, RuntimeLogQuery, RuntimeObservation,
    RuntimeRemoval,
};
use crate::RuntimeResult;
use async_trait::async_trait;

/// Stable provider-neutral lifecycle implemented by every A3S Runtime.
#[async_trait]
pub trait RuntimeClient: Send + Sync {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities>;

    async fn apply(&self, request: &RuntimeApplyRequest) -> RuntimeResult<RuntimeObservation>;

    async fn inspect(&self, unit_id: &str) -> RuntimeResult<RuntimeInspection>;

    async fn stop(&self, request: &RuntimeActionRequest) -> RuntimeResult<RuntimeInspection>;

    async fn remove(&self, request: &RuntimeActionRequest) -> RuntimeResult<RuntimeRemoval>;

    async fn logs(&self, query: &RuntimeLogQuery) -> RuntimeResult<Vec<RuntimeLogChunk>>;

    async fn exec(&self, request: &RuntimeExecRequest) -> RuntimeResult<RuntimeExecResult>;
}
