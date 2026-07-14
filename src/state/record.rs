use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeObservation, RuntimeRemoval, RuntimeUnitSpec,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub request_id: String,
    pub kind: RuntimeRequestKind,
    pub request_digest: String,
    pub state: RuntimeRequestState,
    pub observation: Option<RuntimeObservation>,
    pub removal: Option<RuntimeRemoval>,
}

impl RuntimeRequestReceipt {
    pub(crate) fn pending_apply(request: &RuntimeApplyRequest) -> Result<Self, String> {
        Ok(Self {
            request_id: request.request_id.clone(),
            kind: RuntimeRequestKind::Apply,
            request_digest: request.digest()?,
            state: RuntimeRequestState::Pending,
            observation: None,
            removal: None,
        })
    }

    pub(crate) fn pending_action(
        kind: RuntimeActionKind,
        request: &RuntimeActionRequest,
    ) -> Result<Self, String> {
        Ok(Self {
            request_id: request.request_id.clone(),
            kind: kind.into(),
            request_digest: request.digest()?,
            state: RuntimeRequestState::Pending,
            observation: None,
            removal: None,
        })
    }

    pub(crate) fn complete_with_observation(&mut self, observation: RuntimeObservation) {
        self.state = RuntimeRequestState::Completed;
        self.observation = Some(observation);
        self.removal = None;
    }

    pub(crate) fn complete_with_removal(&mut self, removal: RuntimeRemoval) {
        self.state = RuntimeRequestState::Completed;
        self.observation = None;
        self.removal = Some(removal);
    }

    fn validate(&self) -> Result<(), String> {
        crate::contract::validate_id("request_id", &self.request_id, 512)?;
        crate::contract::validate_digest(&self.request_digest)?;
        match (self.kind, self.state) {
            (_, RuntimeRequestState::Pending)
                if self.observation.is_none() && self.removal.is_none() =>
            {
                Ok(())
            }
            (
                RuntimeRequestKind::Apply | RuntimeRequestKind::Stop,
                RuntimeRequestState::Completed,
            ) if self.observation.is_some() && self.removal.is_none() => {
                self.observation.as_ref().unwrap().validate()
            }
            (RuntimeRequestKind::Remove, RuntimeRequestState::Completed)
                if self.observation.is_none() && self.removal.is_some() =>
            {
                self.removal.as_ref().unwrap().validate()
            }
            _ => Err("Runtime request receipt result does not match its kind and state".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeUnitRecord {
    pub schema: String,
    pub spec: RuntimeUnitSpec,
    pub observation: RuntimeObservation,
    pub removed_at_ms: Option<u64>,
    pub requests: BTreeMap<String, RuntimeRequestReceipt>,
}

impl RuntimeUnitRecord {
    pub const SCHEMA: &'static str = "a3s.runtime.unit-record.v1";

    pub(crate) fn new(request: &RuntimeApplyRequest, now_ms: u64) -> Result<Self, String> {
        let receipt = RuntimeRequestReceipt::pending_apply(request)?;
        let mut requests = BTreeMap::new();
        requests.insert(request.request_id.clone(), receipt);
        let record = Self {
            schema: Self::SCHEMA.into(),
            spec: request.spec.clone(),
            observation: RuntimeObservation::accepted(&request.spec, now_ms)?,
            removed_at_ms: None,
            requests,
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
        self.observation.validate_against(&self.spec)?;
        if self.requests.len() > 10_000 {
            return Err("Runtime unit record exceeds request receipt limit".into());
        }
        for (request_id, receipt) in &self.requests {
            receipt.validate()?;
            if request_id != &receipt.request_id {
                return Err("Runtime request receipt key mismatch".into());
            }
            if let Some(observation) = &receipt.observation {
                if observation.unit_id != self.spec.unit_id {
                    return Err("Runtime request receipt belongs to another unit".into());
                }
            }
            if let Some(removal) = &receipt.removal {
                if removal.unit_id != self.spec.unit_id {
                    return Err("Runtime removal receipt belongs to another unit".into());
                }
            }
        }
        Ok(())
    }

    pub fn receipt(&self, request_id: &str) -> Option<&RuntimeRequestReceipt> {
        self.requests.get(request_id)
    }

    pub(crate) fn receipt_mut(
        &mut self,
        request_id: &str,
    ) -> Result<&mut RuntimeRequestReceipt, String> {
        self.requests
            .get_mut(request_id)
            .ok_or_else(|| format!("Runtime request receipt {request_id:?} was not reserved"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStateReservation {
    pub dispatch: bool,
    pub record: RuntimeUnitRecord,
    pub receipt: RuntimeRequestReceipt,
}
