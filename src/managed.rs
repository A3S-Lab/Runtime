use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeCapabilities, RuntimeExecRequest,
    RuntimeExecResult, RuntimeInspection, RuntimeLogChunk, RuntimeLogQuery, RuntimeObservation,
    RuntimeRemoval, RuntimeUnitState,
};
use crate::{
    RuntimeActionKind, RuntimeClient, RuntimeClock, RuntimeDriver, RuntimeError, RuntimeResult,
    RuntimeStateStore, SystemRuntimeClock,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Shared durable lifecycle implementation used by provider integrations.
pub struct ManagedRuntimeClient {
    state: Arc<dyn RuntimeStateStore>,
    driver: Arc<dyn RuntimeDriver>,
    clock: Arc<dyn RuntimeClock>,
}

impl ManagedRuntimeClient {
    pub fn new(state: Arc<dyn RuntimeStateStore>, driver: Arc<dyn RuntimeDriver>) -> Self {
        Self::with_clock(state, driver, Arc::new(SystemRuntimeClock))
    }

    pub fn with_clock(
        state: Arc<dyn RuntimeStateStore>,
        driver: Arc<dyn RuntimeDriver>,
        clock: Arc<dyn RuntimeClock>,
    ) -> Self {
        Self {
            state,
            driver,
            clock,
        }
    }

    async fn checked_capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        let capabilities = self.driver.capabilities().await?;
        capabilities.validate().map_err(RuntimeError::Protocol)?;
        if &capabilities.provider_id != self.driver.provider_id() {
            return Err(RuntimeError::Protocol(format!(
                "Runtime driver {:?} reported capabilities for {:?}",
                self.driver.provider_id().as_str(),
                capabilities.provider_id.as_str()
            )));
        }
        Ok(capabilities)
    }

    fn check_deadline(&self, deadline_at_ms: Option<u64>) -> RuntimeResult<()> {
        if deadline_at_ms.is_some_and(|deadline| deadline <= self.clock.now_ms()) {
            return Err(RuntimeError::DeadlineExceeded(
                "request expired before provider dispatch".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl RuntimeClient for ManagedRuntimeClient {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        self.checked_capabilities().await
    }

    async fn apply(&self, request: &RuntimeApplyRequest) -> RuntimeResult<RuntimeObservation> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        self.check_deadline(request.deadline_at_ms)?;
        let capabilities = self.checked_capabilities().await?;
        let missing = capabilities
            .missing_for(&request.spec)
            .map_err(RuntimeError::InvalidRequest)?;
        if !missing.is_empty() {
            return Err(RuntimeError::UnsupportedCapabilities(missing));
        }

        let reservation = self
            .state
            .reserve_apply(request, self.clock.now_ms())
            .await?;
        if !reservation.dispatch {
            return reservation.receipt.observation.ok_or_else(|| {
                RuntimeError::Protocol("completed apply receipt has no observation".into())
            });
        }

        let observation = self
            .driver
            .apply(&request.spec, &reservation.record.observation)
            .await?;
        observation
            .validate_against(&request.spec)
            .map_err(RuntimeError::Protocol)?;
        ensure_apply_result(&request.spec, &observation)?;
        Ok(self
            .state
            .update_observation(Some(&request.request_id), &observation)
            .await?
            .observation)
    }

    async fn inspect(&self, unit_id: &str) -> RuntimeResult<RuntimeInspection> {
        let record = match self.state.load(unit_id).await {
            Ok(record) => record,
            Err(RuntimeError::NotFound { .. }) => {
                return Ok(RuntimeInspection::NotFound {
                    schema: RuntimeInspection::SCHEMA.into(),
                    unit_id: unit_id.into(),
                    last_generation: None,
                });
            }
            Err(error) => return Err(error),
        };
        if record.removed_at_ms.is_some() {
            return Ok(RuntimeInspection::NotFound {
                schema: RuntimeInspection::SCHEMA.into(),
                unit_id: unit_id.into(),
                last_generation: Some(record.spec.generation),
            });
        }
        if record.observation.state.is_terminal() {
            return Ok(RuntimeInspection::Found {
                schema: RuntimeInspection::SCHEMA.into(),
                observation: Box::new(record.observation),
            });
        }

        let inspection = self.driver.inspect(&record).await?;
        inspection.validate().map_err(RuntimeError::Protocol)?;
        match inspection {
            RuntimeInspection::Found { observation, .. } => {
                observation
                    .validate_against(&record.spec)
                    .map_err(RuntimeError::Protocol)?;
                let record = self
                    .state
                    .update_observation(None, observation.as_ref())
                    .await?;
                Ok(RuntimeInspection::Found {
                    schema: RuntimeInspection::SCHEMA.into(),
                    observation: Box::new(record.observation),
                })
            }
            RuntimeInspection::NotFound { .. } => {
                let mut unknown = record.observation;
                unknown.state = RuntimeUnitState::Unknown;
                unknown.observed_at_ms = unknown.observed_at_ms.max(self.clock.now_ms());
                unknown.finished_at_ms = None;
                unknown.health = None;
                unknown.outputs.clear();
                unknown.failure = None;
                let record = self.state.update_observation(None, &unknown).await?;
                Ok(RuntimeInspection::Found {
                    schema: RuntimeInspection::SCHEMA.into(),
                    observation: Box::new(record.observation),
                })
            }
        }
    }

    async fn stop(&self, request: &RuntimeActionRequest) -> RuntimeResult<RuntimeInspection> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        self.check_deadline(request.deadline_at_ms)?;
        let capabilities = self.checked_capabilities().await?;
        if !capabilities.supports_feature(crate::contract::RuntimeFeature::Stop) {
            return Err(RuntimeError::UnsupportedCapabilities(vec![
                "feature:Stop".into()
            ]));
        }
        let reservation = self
            .state
            .reserve_action(RuntimeActionKind::Stop, request, self.clock.now_ms())
            .await?;
        if !reservation.dispatch {
            return Ok(RuntimeInspection::Found {
                schema: RuntimeInspection::SCHEMA.into(),
                observation: Box::new(reservation.receipt.observation.ok_or_else(|| {
                    RuntimeError::Protocol("completed stop receipt has no observation".into())
                })?),
            });
        }
        let observation = self.driver.stop(&reservation.record, request).await?;
        observation
            .validate_against(&reservation.record.spec)
            .map_err(RuntimeError::Protocol)?;
        ensure_stop_result(&reservation.record.observation, &observation)?;
        let record = self
            .state
            .update_observation(Some(&request.request_id), &observation)
            .await?;
        Ok(RuntimeInspection::Found {
            schema: RuntimeInspection::SCHEMA.into(),
            observation: Box::new(record.observation),
        })
    }

    async fn remove(&self, request: &RuntimeActionRequest) -> RuntimeResult<RuntimeRemoval> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        self.check_deadline(request.deadline_at_ms)?;
        let capabilities = self.checked_capabilities().await?;
        if !capabilities.supports_feature(crate::contract::RuntimeFeature::Remove) {
            return Err(RuntimeError::UnsupportedCapabilities(vec![
                "feature:Remove".into(),
            ]));
        }
        let reservation = self
            .state
            .reserve_action(RuntimeActionKind::Remove, request, self.clock.now_ms())
            .await?;
        if !reservation.dispatch {
            return reservation.receipt.removal.ok_or_else(|| {
                RuntimeError::Protocol("completed remove receipt has no removal".into())
            });
        }
        let removal = self.driver.remove(&reservation.record, request).await?;
        removal.validate().map_err(RuntimeError::Protocol)?;
        if removal.request_id != request.request_id
            || removal.unit_id != request.unit_id
            || removal.generation != request.generation
        {
            return Err(RuntimeError::Protocol(
                "provider removal changed immutable request identity".into(),
            ));
        }
        self.state.complete_removal(&removal).await?;
        Ok(removal)
    }

    async fn logs(&self, query: &RuntimeLogQuery) -> RuntimeResult<Vec<RuntimeLogChunk>> {
        query.validate().map_err(RuntimeError::InvalidRequest)?;
        let capabilities = self.checked_capabilities().await?;
        if !capabilities.supports_feature(crate::contract::RuntimeFeature::Logs) {
            return Err(RuntimeError::UnsupportedCapabilities(vec![
                "feature:Logs".into()
            ]));
        }
        let record = self.state.load(&query.unit_id).await?;
        ensure_current_generation(&record, query.generation)?;
        let chunks = self.driver.logs(&record, query).await?;
        for chunk in &chunks {
            chunk.validate().map_err(RuntimeError::Protocol)?;
        }
        if chunks
            .windows(2)
            .any(|pair| pair[0].sequence >= pair[1].sequence)
        {
            return Err(RuntimeError::Protocol(
                "provider returned unordered log chunks".into(),
            ));
        }
        Ok(chunks)
    }

    async fn exec(&self, request: &RuntimeExecRequest) -> RuntimeResult<RuntimeExecResult> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        let capabilities = self.checked_capabilities().await?;
        if !capabilities.supports_feature(crate::contract::RuntimeFeature::Exec) {
            return Err(RuntimeError::UnsupportedCapabilities(vec![
                "feature:Exec".into()
            ]));
        }
        let record = self.state.load(&request.unit_id).await?;
        ensure_current_generation(&record, request.generation)?;
        if record.observation.state != RuntimeUnitState::Running {
            return Err(RuntimeError::InvalidRequest(format!(
                "Runtime exec requires a running unit; {:?} is {:?}",
                request.unit_id, record.observation.state
            )));
        }
        let result = self.driver.exec(&record, request).await?;
        result.validate().map_err(RuntimeError::Protocol)?;
        result
            .observation
            .validate_against(&record.spec)
            .map_err(RuntimeError::Protocol)?;
        if result.request_id != request.request_id
            || result.observation.unit_id != request.unit_id
            || result.observation.generation != request.generation
        {
            return Err(RuntimeError::Protocol(
                "provider exec changed immutable request identity".into(),
            ));
        }
        Ok(result)
    }
}

fn ensure_current_generation(
    record: &crate::RuntimeUnitRecord,
    requested: u64,
) -> RuntimeResult<()> {
    if record.removed_at_ms.is_some() {
        return Err(RuntimeError::NotFound {
            unit_id: record.spec.unit_id.clone(),
        });
    }
    if requested < record.spec.generation {
        return Err(RuntimeError::StaleGeneration {
            unit_id: record.spec.unit_id.clone(),
            requested,
            current: record.spec.generation,
        });
    }
    if requested != record.spec.generation {
        return Err(RuntimeError::GenerationConflict {
            unit_id: record.spec.unit_id.clone(),
            generation: requested,
        });
    }
    Ok(())
}

fn ensure_apply_result(
    spec: &crate::contract::RuntimeUnitSpec,
    observation: &RuntimeObservation,
) -> RuntimeResult<()> {
    let allowed = match spec.class {
        crate::contract::RuntimeUnitClass::Task => matches!(
            observation.state,
            RuntimeUnitState::Succeeded | RuntimeUnitState::Failed
        ),
        crate::contract::RuntimeUnitClass::Service => matches!(
            observation.state,
            RuntimeUnitState::Running
                | RuntimeUnitState::Stopped
                | RuntimeUnitState::Failed
                | RuntimeUnitState::Unknown
        ),
    };
    if !allowed {
        return Err(RuntimeError::Protocol(format!(
            "provider apply returned invalid {:?} result {:?}",
            spec.class, observation.state
        )));
    }
    if observation.provider_resource_id.is_none() || observation.provider_build.is_none() {
        return Err(RuntimeError::Protocol(
            "provider apply returned an observation without provider identity".into(),
        ));
    }
    Ok(())
}

fn ensure_stop_result(
    current: &RuntimeObservation,
    observation: &RuntimeObservation,
) -> RuntimeResult<()> {
    if observation.state == RuntimeUnitState::Stopped
        || observation.state == RuntimeUnitState::Unknown
        || current.state.is_terminal() && observation == current
    {
        return Ok(());
    }
    Err(RuntimeError::Protocol(format!(
        "provider stop returned nonterminal state {:?}",
        observation.state
    )))
}
