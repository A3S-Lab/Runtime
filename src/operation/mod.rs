mod file;
mod record;

use crate::contract::{RuntimeExecutionResult, RuntimeExecutionSpec};
use crate::RuntimeResult;
use async_trait::async_trait;

pub use file::FileOperationStore;
pub use record::{OperationRecord, OperationReservation};

#[async_trait]
pub trait OperationStore: Send + Sync {
    async fn reserve(&self, spec: &RuntimeExecutionSpec) -> RuntimeResult<OperationReservation>;

    async fn load(&self, operation_id: &str) -> RuntimeResult<OperationRecord>;

    async fn update(&self, result: &RuntimeExecutionResult) -> RuntimeResult<OperationRecord>;
}
