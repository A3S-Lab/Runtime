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
/// A successful `apply` must also retire every older provider generation for
/// the unit before returning, leaving exactly one managed provider resource.
/// If that handoff is interrupted, an exact retry must discover the partially
/// created current generation and finish the same reconciliation.
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

    /// Executes one durably identified request within its original budget.
    ///
    /// `ManagedRuntimeClient` always supplies `request.deadline_at_ms` as the
    /// effective absolute deadline captured by the first reservation: the
    /// smaller of that attempt's `timeout_ms` window and any caller-provided
    /// absolute deadline. A pending replay receives the same persisted value,
    /// so a driver must not restart or extend the execution window. Drivers may
    /// enforce a shorter provider-specific timeout and must deduplicate or
    /// reattach the stable request ID after an ambiguous result.
    async fn exec(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeExecRequest,
    ) -> RuntimeResult<RuntimeExecResult>;
}
