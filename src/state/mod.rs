mod file;
mod record;

use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeObservation, RuntimeRemoval,
};
use crate::RuntimeResult;
use async_trait::async_trait;

pub use file::FileRuntimeStateStore;
pub use record::{
    RuntimeActionKind, RuntimeRequestKind, RuntimeRequestReceipt, RuntimeRequestState,
    RuntimeStateReservation, RuntimeUnitRecord,
};

#[async_trait]
pub trait RuntimeStateStore: Send + Sync {
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

    async fn load(&self, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord>;

    async fn update_observation(
        &self,
        request_id: Option<&str>,
        observation: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeUnitRecord>;

    async fn complete_removal(&self, removal: &RuntimeRemoval) -> RuntimeResult<RuntimeUnitRecord>;
}
