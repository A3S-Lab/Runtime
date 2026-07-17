use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeExecRequest, RuntimeExecResult,
    RuntimeObservation, RuntimeRemoval, RuntimeUnitSpec,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeActionKind {
    Stop,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRequestKind {
    Apply,
    Stop,
    Remove,
    Exec,
}

impl From<RuntimeActionKind> for RuntimeRequestKind {
    fn from(value: RuntimeActionKind) -> Self {
        match value {
            RuntimeActionKind::Stop => Self::Stop,
            RuntimeActionKind::Remove => Self::Remove,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRequestState {
    Pending,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequestReceipt {
    pub schema: String,
    pub request_id: String,
    pub unit_id: String,
    pub generation: u64,
    pub kind: RuntimeRequestKind,
    pub request_digest: String,
    pub state: RuntimeRequestState,
    pub observation: Option<RuntimeObservation>,
    pub removal: Option<RuntimeRemoval>,
    pub exec_result: Option<RuntimeExecResult>,
}

impl RuntimeRequestReceipt {
    pub const SCHEMA: &'static str = "a3s.runtime.request-receipt.v1";

    pub(crate) fn pending_apply(request: &RuntimeApplyRequest) -> Result<Self, String> {
        Ok(Self::pending(
            request.request_id.clone(),
            request.spec.unit_id.clone(),
            request.spec.generation,
            RuntimeRequestKind::Apply,
            request.digest()?,
        ))
    }

    pub(crate) fn pending_action(
        kind: RuntimeActionKind,
        request: &RuntimeActionRequest,
    ) -> Result<Self, String> {
        Ok(Self::pending(
            request.request_id.clone(),
            request.unit_id.clone(),
            request.generation,
            kind.into(),
            request.digest()?,
        ))
    }

    pub(crate) fn pending_exec(request: &RuntimeExecRequest) -> Result<Self, String> {
        Ok(Self::pending(
            request.request_id.clone(),
            request.unit_id.clone(),
            request.generation,
            RuntimeRequestKind::Exec,
            request.digest()?,
        ))
    }

    fn pending(
        request_id: String,
        unit_id: String,
        generation: u64,
        kind: RuntimeRequestKind,
        request_digest: String,
    ) -> Self {
        Self {
            schema: Self::SCHEMA.into(),
            request_id,
            unit_id,
            generation,
            kind,
            request_digest,
            state: RuntimeRequestState::Pending,
            observation: None,
            removal: None,
            exec_result: None,
        }
    }

    pub(crate) fn complete_with_observation(&mut self, observation: RuntimeObservation) {
        self.state = RuntimeRequestState::Completed;
        self.observation = Some(observation);
        self.removal = None;
        self.exec_result = None;
    }

    pub(crate) fn complete_with_removal(&mut self, removal: RuntimeRemoval) {
        self.state = RuntimeRequestState::Completed;
        self.observation = None;
        self.removal = Some(removal);
        self.exec_result = None;
    }

    pub(crate) fn complete_with_exec_result(&mut self, result: RuntimeExecResult) {
        self.state = RuntimeRequestState::Completed;
        self.observation = None;
        self.removal = None;
        self.exec_result = Some(result);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime request receipt schema {:?}",
                self.schema
            ));
        }
        crate::contract::validate_id("request_id", &self.request_id, 512)?;
        crate::contract::validate_id("unit_id", &self.unit_id, 512)?;
        if self.generation == 0 {
            return Err("Runtime request receipt generation must be positive".into());
        }
        crate::contract::validate_digest(&self.request_digest)?;
        match (
            self.kind,
            self.state,
            &self.observation,
            &self.removal,
            &self.exec_result,
        ) {
            (_, RuntimeRequestState::Pending, None, None, None) => Ok(()),
            (
                RuntimeRequestKind::Apply | RuntimeRequestKind::Stop,
                RuntimeRequestState::Completed,
                Some(observation),
                None,
                None,
            ) => {
                observation.validate()?;
                self.validate_unit_result(&observation.unit_id, observation.generation)
            }
            (
                RuntimeRequestKind::Remove,
                RuntimeRequestState::Completed,
                None,
                Some(removal),
                None,
            ) => {
                removal.validate()?;
                if removal.request_id != self.request_id {
                    return Err("Runtime removal receipt request identity mismatch".into());
                }
                self.validate_unit_result(&removal.unit_id, removal.generation)
            }
            (
                RuntimeRequestKind::Exec,
                RuntimeRequestState::Completed,
                None,
                None,
                Some(result),
            ) => {
                result.validate()?;
                if result.request_id != self.request_id {
                    return Err("Runtime exec receipt request identity mismatch".into());
                }
                self.validate_unit_result(
                    &result.observation.unit_id,
                    result.observation.generation,
                )
            }
            _ => Err("Runtime request receipt result does not match its kind and state".into()),
        }
    }

    fn validate_unit_result(&self, unit_id: &str, generation: u64) -> Result<(), String> {
        if unit_id != self.unit_id || generation != self.generation {
            return Err("Runtime request receipt result identity mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeUnitRecord {
    pub schema: String,
    pub spec: RuntimeUnitSpec,
    pub observation: RuntimeObservation,
    pub removed_at_ms: Option<u64>,
}

impl RuntimeUnitRecord {
    pub const SCHEMA: &'static str = "a3s.runtime.unit-record.v2";

    pub(crate) fn new(request: &RuntimeApplyRequest, now_ms: u64) -> Result<Self, String> {
        let record = Self {
            schema: Self::SCHEMA.into(),
            spec: request.spec.clone(),
            observation: RuntimeObservation::accepted(&request.spec, now_ms)?,
            removed_at_ms: None,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime unit record schema {:?}",
                self.schema
            ));
        }
        self.spec.validate()?;
        self.observation.validate_against(&self.spec)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStateReservation {
    pub dispatch: bool,
    pub record: RuntimeUnitRecord,
    pub receipt: RuntimeRequestReceipt,
}
