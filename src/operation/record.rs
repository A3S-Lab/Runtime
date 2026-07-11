use crate::contract::{ExecutionState, RuntimeExecutionResult, RuntimeExecutionSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub schema: String,
    pub spec: RuntimeExecutionSpec,
    pub result: RuntimeExecutionResult,
}

impl OperationRecord {
    pub const SCHEMA: &'static str = "a3s.runtime.operation-record.v1";

    pub(crate) fn queued(spec: RuntimeExecutionSpec) -> Result<Self, String> {
        let spec_digest = spec.digest()?;
        let execution_id = super::file::execution_id(&spec.operation_id);
        let result = RuntimeExecutionResult {
            schema: RuntimeExecutionResult::SCHEMA.into(),
            execution_id,
            operation_id: spec.operation_id.clone(),
            spec_digest,
            role: spec.role,
            state: ExecutionState::Queued,
            started_at_ms: None,
            finished_at_ms: None,
            typed_result_artifact: None,
            terminal_checkpoint: None,
            submission_snapshot: None,
            usage: None,
            evidence: None,
            provider_attestation: None,
            failure: None,
        };
        let record = Self {
            schema: Self::SCHEMA.into(),
            spec,
            result,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported operation record schema {:?}",
                self.schema
            ));
        }
        self.spec.validate()?;
        self.result.validate()?;
        if self.spec.operation_id != self.result.operation_id
            || self.spec.role != self.result.role
            || self.spec.digest()? != self.result.spec_digest
        {
            return Err("operation record identity mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationReservation {
    pub created: bool,
    pub record: OperationRecord,
}
