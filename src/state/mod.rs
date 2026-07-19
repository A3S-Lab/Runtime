mod file;
mod record;

use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeExecRequest, RuntimeExecResult,
    RuntimeObservation, RuntimeRemoval,
};
use crate::RuntimeResult;
use async_trait::async_trait;

pub use file::FileRuntimeStateStore;
pub use record::{
    RuntimeActionKind, RuntimeRequestKind, RuntimeRequestReceipt, RuntimeRequestState,
    RuntimeStateReservation, RuntimeUnitRecord,
};

#[cfg(test)]
mod transition_tests;

/// Owned guard for one unit's cross-process operation lease. Implementations
/// release the lease when the guard is dropped.
pub trait RuntimeOperationLease: Send {}

#[async_trait]
pub trait RuntimeStateStore: Send + Sync {
    async fn acquire_operation_lease(
        &self,
        unit_id: &str,
    ) -> RuntimeResult<Box<dyn RuntimeOperationLease>>;

    async fn reserve_apply(
        &self,
        request: &RuntimeApplyRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation>;

    async fn reserve_action(
        &self,
        kind: RuntimeActionKind,
        request: &RuntimeActionRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation>;

    async fn reserve_exec(
        &self,
        request: &RuntimeExecRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation>;

    async fn load(&self, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord>;

    async fn load_request(
        &self,
        unit_id: &str,
        request_id: &str,
    ) -> RuntimeResult<RuntimeRequestReceipt>;

    async fn update_observation(
        &self,
        request_id: Option<&str>,
        observation: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeUnitRecord>;

    async fn complete_removal(&self, removal: &RuntimeRemoval) -> RuntimeResult<RuntimeUnitRecord>;

    async fn complete_exec(&self, result: &RuntimeExecResult) -> RuntimeResult<RuntimeUnitRecord>;
}
