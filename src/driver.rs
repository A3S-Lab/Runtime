use crate::contract::{
    RuntimeActionRequest, RuntimeCapabilities, RuntimeExecRequest, RuntimeExecResult,
    RuntimeInspection, RuntimeLogChunk, RuntimeLogQuery, RuntimeObservation, RuntimeRemoval,
    RuntimeUnitSpec,
};
use crate::{ProviderId, RuntimeResult, RuntimeUnitRecord};
use async_trait::async_trait;

/// Provider-specific primitive used by `ManagedRuntimeClient`.
///
/// Drivers do not own request idempotency or durable shared state. `apply`,
/// `stop`, and `remove` must nevertheless be safe to reattach after an
/// ambiguous transport failure using the stable unit identity and generation.
#[async_trait]
pub trait RuntimeDriver: Send + Sync {
    fn provider_id(&self) -> &ProviderId;

    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities>;

    async fn apply(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeObservation>;

    async fn inspect(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection>;

    async fn stop(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeObservation>;

    async fn remove(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeRemoval>;

    async fn logs(
        &self,
        unit: &RuntimeUnitRecord,
        query: &RuntimeLogQuery,
    ) -> RuntimeResult<Vec<RuntimeLogChunk>>;

    async fn exec(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeExecRequest,
    ) -> RuntimeResult<RuntimeExecResult>;
}
