#[cfg(test)]
use self::fs::test_failpoint;
use self::fs::{
    atomic_write, ensure_directory, io_error, owner_only_open, path_exists, read_optional_receipt,
    read_optional_record, read_required_receipt, read_required_record,
};
use super::{
    RuntimeActionKind, RuntimeOperationLease, RuntimeRequestKind, RuntimeRequestReceipt,
    RuntimeRequestState, RuntimeStateReservation, RuntimeStateStore, RuntimeUnitRecord,
};
use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeExecRequest, RuntimeExecResult,
    RuntimeObservation, RuntimeRemoval, RuntimeUnitState,
};
use crate::{RuntimeError, RuntimeResult};
use async_trait::async_trait;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::path::PathBuf;

mod fs;

#[derive(Debug, Clone)]
pub struct FileRuntimeStateStore {
    root: PathBuf,
}

impl FileRuntimeStateStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn acquire_operation_lease_sync(&self, unit_id: &str) -> RuntimeResult<FileOperationLease> {
        validate_unit_id(unit_id)?;
        ensure_directory(&self.root)?;
        let operations = self.root.join("operations");
        ensure_directory(&operations)?;
        let path = operations.join(format!("{}.lock", storage_key(unit_id)));
        let file = owner_only_open(&path, "Runtime operation lease")?;
        file.lock_exclusive()
            .map_err(io_error("lock Runtime operation"))?;
        Ok(FileOperationLease(file))
    }

    fn reserve_apply_sync(
        &self,
        request: RuntimeApplyRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        let _lock = self.lock(&request.spec.unit_id)?;
        let record_path = self.record_path(&request.spec.unit_id, true)?;
        let mut receipt_path =
            self.request_path(&request.spec.unit_id, &request.request_id, false)?;
        let existing = read_optional_receipt(&receipt_path)?;
        if let Some(receipt) = &existing {
            let digest = request.digest().map_err(RuntimeError::InvalidRequest)?;
            ensure_same_request(receipt, RuntimeRequestKind::Apply, &digest)?;
            ensure_receipt_target(receipt, &request.spec.unit_id, request.spec.generation)?;
        }

        let stored_record = read_optional_record(&record_path)?;
        if stored_record.is_none()
            && existing
                .as_ref()
                .is_some_and(|receipt| receipt.state == RuntimeRequestState::Completed)
        {
            return Err(RuntimeError::Protocol(
                "completed Runtime apply receipt has no unit record".into(),
            ));
        }
        let mut record_changed = stored_record.is_none();
        let mut record = match stored_record {
            Some(record) => record,
            None => RuntimeUnitRecord::new(&request, now_ms).map_err(RuntimeError::Protocol)?,
        };

        if let Some(receipt) = existing
            .as_ref()
            .filter(|receipt| receipt.state == RuntimeRequestState::Completed)
        {
            if reconcile_completed_observation(&mut record, receipt)? {
                atomic_write(&record_path, &record, "state record")?;
            }
            return Ok(reservation(record, receipt.clone()));
        }

        let current_generation = record.spec.generation;
        if request.spec.generation < current_generation {
            return Err(RuntimeError::StaleGeneration {
                unit_id: request.spec.unit_id,
                requested: request.spec.generation,
                current: current_generation,
            });
        }

        let receipt_is_new = existing.is_none();
        let mut receipt = existing.unwrap_or(
            RuntimeRequestReceipt::pending_apply(&request).map_err(RuntimeError::Protocol)?,
        );

        if request.spec.generation == current_generation {
            if request
                .spec
                .digest()
                .map_err(RuntimeError::InvalidRequest)?
                != record.spec.digest().map_err(RuntimeError::Protocol)?
                || record.removed_at_ms.is_some()
            {
                return Err(RuntimeError::GenerationConflict {
                    unit_id: request.spec.unit_id,
                    generation: request.spec.generation,
                });
            }
            if !matches!(
                record.observation.state,
                RuntimeUnitState::Accepted | RuntimeUnitState::Unknown
            ) {
                receipt.complete_with_observation(record.observation.clone());
                receipt.validate().map_err(RuntimeError::Protocol)?;
            }
        } else {
            record.spec = request.spec.clone();
            record.observation = RuntimeObservation::accepted(&request.spec, now_ms)
                .map_err(RuntimeError::Protocol)?;
            record.removed_at_ms = None;
            record_changed = true;
        }

        if receipt_is_new || receipt.state == RuntimeRequestState::Completed {
            if receipt_is_new {
                receipt_path =
                    self.request_path(&request.spec.unit_id, &request.request_id, true)?;
            }
            atomic_write(&receipt_path, &receipt, "request receipt")?;
        }
        record.validate().map_err(RuntimeError::Protocol)?;
        if record_changed {
            atomic_write(&record_path, &record, "state record")?;
        }
        Ok(reservation(record, receipt))
    }

    fn reserve_action_sync(
        &self,
        kind: RuntimeActionKind,
        request: RuntimeActionRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        let _lock = self.lock(&request.unit_id)?;
        let record_path = self.record_path(&request.unit_id, false)?;
        let mut record = read_required_record(&record_path, &request.unit_id)?;
        let mut receipt_path = self.request_path(&request.unit_id, &request.request_id, false)?;
        if let Some(receipt) = read_optional_receipt(&receipt_path)? {
            let digest = request.digest().map_err(RuntimeError::InvalidRequest)?;
            ensure_same_request(&receipt, kind.into(), &digest)?;
            ensure_receipt_target(&receipt, &request.unit_id, request.generation)?;
            if receipt.state == RuntimeRequestState::Completed {
                let changed = match receipt.kind {
                    RuntimeRequestKind::Stop => {
                        reconcile_completed_observation(&mut record, &receipt)?
                    }
                    RuntimeRequestKind::Remove => {
                        reconcile_completed_removal(&mut record, &receipt)?
                    }
                    _ => false,
                };
                if changed {
                    atomic_write(&record_path, &record, "state record")?;
                }
            } else {
                ensure_current_generation(&record, &request.unit_id, request.generation)?;
                if record.removed_at_ms.is_some() {
                    return Err(RuntimeError::NotFound {
                        unit_id: request.unit_id,
                    });
                }
            }
            return Ok(reservation(record, receipt));
        }

        ensure_current_generation(&record, &request.unit_id, request.generation)?;
        let mut receipt = RuntimeRequestReceipt::pending_action(kind, &request)
            .map_err(RuntimeError::Protocol)?;
        match kind {
            RuntimeActionKind::Stop => {
                if record.removed_at_ms.is_some() {
                    return Err(RuntimeError::NotFound {
                        unit_id: request.unit_id,
                    });
                }
                if record.observation.state.is_terminal() {
                    receipt.complete_with_observation(record.observation.clone());
                }
            }
            RuntimeActionKind::Remove => {
                if record.removed_at_ms.is_some() {
                    receipt.complete_with_removal(RuntimeRemoval {
                        schema: RuntimeRemoval::SCHEMA.into(),
                        request_id: request.request_id.clone(),
                        unit_id: request.unit_id.clone(),
                        generation: request.generation,
                        removed_at_ms: now_ms,
                        already_absent: true,
                    });
                }
            }
        }
        receipt.validate().map_err(RuntimeError::Protocol)?;
        receipt_path = self.request_path(&request.unit_id, &request.request_id, true)?;
        atomic_write(&receipt_path, &receipt, "request receipt")?;
        Ok(reservation(record, receipt))
    }

    fn reserve_exec_sync(
        &self,
        request: RuntimeExecRequest,
        started_at_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        let _lock = self.lock(&request.unit_id)?;
        let record_path = self.record_path(&request.unit_id, false)?;
        let mut record = read_required_record(&record_path, &request.unit_id)?;
        let mut receipt_path = self.request_path(&request.unit_id, &request.request_id, false)?;
        if let Some(receipt) = read_optional_receipt(&receipt_path)? {
            let digest = request.digest().map_err(RuntimeError::InvalidRequest)?;
            ensure_same_request(&receipt, RuntimeRequestKind::Exec, &digest)?;
            ensure_receipt_target(&receipt, &request.unit_id, request.generation)?;
            if receipt.state == RuntimeRequestState::Completed
                && reconcile_completed_exec(&mut record, &receipt)?
            {
                atomic_write(&record_path, &record, "state record")?;
            } else if receipt.state == RuntimeRequestState::Pending {
                ensure_current_generation(&record, &request.unit_id, request.generation)?;
                if record.removed_at_ms.is_some() {
                    return Err(RuntimeError::NotFound {
                        unit_id: request.unit_id,
                    });
                }
            }
            return Ok(reservation(record, receipt));
        }

        ensure_current_generation(&record, &request.unit_id, request.generation)?;
        if record.removed_at_ms.is_some() {
            return Err(RuntimeError::NotFound {
                unit_id: request.unit_id,
            });
        }
        if record.observation.state != RuntimeUnitState::Running {
            return Err(RuntimeError::InvalidRequest(format!(
                "Runtime exec requires a running unit; {:?} is {:?}",
                request.unit_id, record.observation.state
            )));
        }
        let receipt = RuntimeRequestReceipt::pending_exec(&request, started_at_ms)
            .map_err(RuntimeError::Protocol)?;
        receipt.validate().map_err(RuntimeError::Protocol)?;
        receipt_path = self.request_path(&request.unit_id, &request.request_id, true)?;
        atomic_write(&receipt_path, &receipt, "request receipt")?;
        Ok(reservation(record, receipt))
    }

    fn load_sync(&self, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord> {
        validate_unit_id(unit_id)?;
        let _lock = self.lock(unit_id)?;
        let path = self.record_path(unit_id, false)?;
        read_required_record(&path, unit_id)
    }

    fn load_request_sync(
        &self,
        unit_id: &str,
        request_id: &str,
    ) -> RuntimeResult<RuntimeRequestReceipt> {
        validate_unit_id(unit_id)?;
        validate_request_id(request_id)?;
        let _lock = self.lock(unit_id)?;
        let path = self.request_path(unit_id, request_id, false)?;
        let receipt =
            read_optional_receipt(&path)?.ok_or_else(|| RuntimeError::RequestNotFound {
                unit_id: unit_id.into(),
                request_id: request_id.into(),
            })?;
        if receipt.unit_id != unit_id || receipt.request_id != request_id {
            return Err(RuntimeError::Protocol(
                "Runtime request receipt storage key mismatch".into(),
            ));
        }
        Ok(receipt)
    }

    fn update_observation_sync(
        &self,
        request_id: Option<String>,
        observation: RuntimeObservation,
    ) -> RuntimeResult<RuntimeUnitRecord> {
        observation.validate().map_err(RuntimeError::Protocol)?;
        let _lock = self.lock(&observation.unit_id)?;
        let record_path = self.record_path(&observation.unit_id, false)?;
        let mut record = read_required_record(&record_path, &observation.unit_id)?;
        if record.removed_at_ms.is_some() {
            return Err(RuntimeError::Protocol(
                "cannot update an explicitly removed Runtime unit".into(),
            ));
        }
        validate_transition(&record.observation, &observation, &record.spec)?;

        if let Some(request_id) = request_id {
            let receipt_path = self.request_path(&observation.unit_id, &request_id, false)?;
            let mut receipt =
                read_required_receipt(&receipt_path, &observation.unit_id, &request_id)?;
            if !matches!(
                receipt.kind,
                RuntimeRequestKind::Apply | RuntimeRequestKind::Stop
            ) {
                return Err(RuntimeError::Protocol(
                    "request kind cannot complete with an observation".into(),
                ));
            }
            ensure_receipt_target(&receipt, &observation.unit_id, observation.generation)?;
            if receipt.state == RuntimeRequestState::Completed {
                if receipt.observation.as_ref() != Some(&observation) {
                    return Err(RuntimeError::Protocol(
                        "completed Runtime request result changed".into(),
                    ));
                }
            } else {
                receipt.complete_with_observation(observation.clone());
                receipt.validate().map_err(RuntimeError::Protocol)?;
                atomic_write(&receipt_path, &receipt, "request receipt")?;
                #[cfg(test)]
                test_failpoint("state.complete-observation.after-receipt-publish");
            }
        }

        record.observation = observation;
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&record_path, &record, "state record")?;
        Ok(record)
    }

    fn complete_removal_sync(&self, removal: RuntimeRemoval) -> RuntimeResult<RuntimeUnitRecord> {
        removal.validate().map_err(RuntimeError::Protocol)?;
        let _lock = self.lock(&removal.unit_id)?;
        let record_path = self.record_path(&removal.unit_id, false)?;
        let mut record = read_required_record(&record_path, &removal.unit_id)?;
        if removal.generation != record.spec.generation {
            return Err(RuntimeError::Protocol(
                "Runtime removal generation does not match stored unit".into(),
            ));
        }
        let receipt_path = self.request_path(&removal.unit_id, &removal.request_id, false)?;
        let mut receipt =
            read_required_receipt(&receipt_path, &removal.unit_id, &removal.request_id)?;
        if receipt.kind != RuntimeRequestKind::Remove {
            return Err(RuntimeError::Protocol(
                "non-remove request completed with removal result".into(),
            ));
        }
        ensure_receipt_target(&receipt, &removal.unit_id, removal.generation)?;
        if receipt.state == RuntimeRequestState::Completed {
            if receipt.removal.as_ref() != Some(&removal) {
                return Err(RuntimeError::Protocol(
                    "completed Runtime removal result changed".into(),
                ));
            }
        } else {
            receipt.complete_with_removal(removal.clone());
            receipt.validate().map_err(RuntimeError::Protocol)?;
            atomic_write(&receipt_path, &receipt, "request receipt")?;
            #[cfg(test)]
            test_failpoint("state.complete-removal.after-receipt-publish");
        }
        record.removed_at_ms = Some(removal.removed_at_ms);
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&record_path, &record, "state record")?;
        Ok(record)
    }

    fn complete_exec_sync(&self, result: RuntimeExecResult) -> RuntimeResult<RuntimeUnitRecord> {
        result.validate().map_err(RuntimeError::Protocol)?;
        let unit_id = result.observation.unit_id.clone();
        let _lock = self.lock(&unit_id)?;
        let record_path = self.record_path(&unit_id, false)?;
        let mut record = read_required_record(&record_path, &unit_id)?;
        if record.removed_at_ms.is_some() {
            return Err(RuntimeError::Protocol(
                "cannot complete exec for an explicitly removed Runtime unit".into(),
            ));
        }
        result
            .observation
            .validate_against(&record.spec)
            .map_err(RuntimeError::Protocol)?;
        validate_transition(&record.observation, &result.observation, &record.spec)?;
        let receipt_path = self.request_path(&unit_id, &result.request_id, false)?;
        let mut receipt = read_required_receipt(&receipt_path, &unit_id, &result.request_id)?;
        if receipt.kind != RuntimeRequestKind::Exec {
            return Err(RuntimeError::Protocol(
                "non-exec request completed with exec result".into(),
            ));
        }
        if receipt.state == RuntimeRequestState::Completed {
            if receipt.exec_result.as_ref() != Some(&result) {
                return Err(RuntimeError::Protocol(
                    "completed Runtime exec result changed".into(),
                ));
            }
        } else {
            receipt.complete_with_exec_result(result.clone());
            receipt.validate().map_err(RuntimeError::Protocol)?;
            atomic_write(&receipt_path, &receipt, "request receipt")?;
            #[cfg(test)]
            test_failpoint("state.complete-exec.after-receipt-publish");
        }
        record.observation = result.observation;
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&record_path, &record, "state record")?;
        Ok(record)
    }

    fn lock(&self, unit_id: &str) -> RuntimeResult<StateLock> {
        validate_unit_id(unit_id)?;
        ensure_directory(&self.root)?;
        let locks = self.root.join("locks");
        ensure_directory(&locks)?;
        let file = owner_only_open(
            &locks.join(format!("{}.lock", storage_key(unit_id))),
            "Runtime state lock",
        )?;
        file.lock_exclusive()
            .map_err(io_error("lock Runtime unit"))?;
        Ok(StateLock(file))
    }

    fn unit_directory(&self, unit_id: &str, create: bool) -> RuntimeResult<PathBuf> {
        validate_unit_id(unit_id)?;
        let units = self.root.join("units");
        if create {
            ensure_directory(&self.root)?;
            ensure_directory(&units)?;
        } else if path_exists(&units)? {
            ensure_directory(&units)?;
        }
        let key = storage_key(unit_id);
        let legacy = units.join(format!("{key}.json"));
        if path_exists(&legacy)? {
            return Err(RuntimeError::Protocol(format!(
                "legacy Runtime unit record {} requires explicit migration",
                legacy.display()
            )));
        }
        let unit = units.join(key);
        if create || path_exists(&unit)? {
            ensure_directory(&unit)?;
        }
        Ok(unit)
    }

    fn record_path(&self, unit_id: &str, create: bool) -> RuntimeResult<PathBuf> {
        Ok(self.unit_directory(unit_id, create)?.join("record.json"))
    }

    fn request_path(
        &self,
        unit_id: &str,
        request_id: &str,
        create: bool,
    ) -> RuntimeResult<PathBuf> {
        validate_request_id(request_id)?;
        let requests = self.unit_directory(unit_id, create)?.join("requests");
        if create || path_exists(&requests)? {
            ensure_directory(&requests)?;
        }
        Ok(requests.join(format!("{}.json", storage_key(request_id))))
    }
}

#[async_trait]
impl RuntimeStateStore for FileRuntimeStateStore {
    async fn acquire_operation_lease(
        &self,
        unit_id: &str,
    ) -> RuntimeResult<Box<dyn RuntimeOperationLease>> {
        let store = self.clone();
        let unit_id = unit_id.to_owned();
        let lease =
            tokio::task::spawn_blocking(move || store.acquire_operation_lease_sync(&unit_id))
                .await
                .map_err(task_error)??;
        Ok(Box::new(lease))
    }

    async fn reserve_apply(
        &self,
        request: &RuntimeApplyRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        let store = self.clone();
        let request = request.clone();
        tokio::task::spawn_blocking(move || store.reserve_apply_sync(request, now_ms))
            .await
            .map_err(task_error)?
    }

    async fn reserve_action(
        &self,
        kind: RuntimeActionKind,
        request: &RuntimeActionRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        let store = self.clone();
        let request = request.clone();
        tokio::task::spawn_blocking(move || store.reserve_action_sync(kind, request, now_ms))
            .await
            .map_err(task_error)?
    }

    async fn reserve_exec(
        &self,
        request: &RuntimeExecRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        let store = self.clone();
        let request = request.clone();
        tokio::task::spawn_blocking(move || store.reserve_exec_sync(request, now_ms))
            .await
            .map_err(task_error)?
    }

    async fn load(&self, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord> {
        let store = self.clone();
        let unit_id = unit_id.to_owned();
        tokio::task::spawn_blocking(move || store.load_sync(&unit_id))
            .await
            .map_err(task_error)?
    }

    async fn load_request(
        &self,
        unit_id: &str,
        request_id: &str,
    ) -> RuntimeResult<RuntimeRequestReceipt> {
        let store = self.clone();
        let unit_id = unit_id.to_owned();
        let request_id = request_id.to_owned();
        tokio::task::spawn_blocking(move || store.load_request_sync(&unit_id, &request_id))
            .await
            .map_err(task_error)?
    }

    async fn update_observation(
        &self,
        request_id: Option<&str>,
        observation: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeUnitRecord> {
        let store = self.clone();
        let request_id = request_id.map(str::to_owned);
        let observation = observation.clone();
        tokio::task::spawn_blocking(move || store.update_observation_sync(request_id, observation))
            .await
            .map_err(task_error)?
    }

    async fn complete_removal(&self, removal: &RuntimeRemoval) -> RuntimeResult<RuntimeUnitRecord> {
        let store = self.clone();
        let removal = removal.clone();
        tokio::task::spawn_blocking(move || store.complete_removal_sync(removal))
            .await
            .map_err(task_error)?
    }

    async fn complete_exec(&self, result: &RuntimeExecResult) -> RuntimeResult<RuntimeUnitRecord> {
        let store = self.clone();
        let result = result.clone();
        tokio::task::spawn_blocking(move || store.complete_exec_sync(result))
            .await
            .map_err(task_error)?
    }
}

fn reservation(
    record: RuntimeUnitRecord,
    receipt: RuntimeRequestReceipt,
) -> RuntimeStateReservation {
    RuntimeStateReservation {
        dispatch: receipt.state == RuntimeRequestState::Pending,
        record,
        receipt,
    }
}

fn ensure_same_request(
    receipt: &RuntimeRequestReceipt,
    kind: RuntimeRequestKind,
    digest: &str,
) -> RuntimeResult<()> {
    if receipt.kind != kind || receipt.request_digest != digest {
        return Err(RuntimeError::RequestConflict {
            request_id: receipt.request_id.clone(),
        });
    }
    Ok(())
}

fn ensure_receipt_target(
    receipt: &RuntimeRequestReceipt,
    unit_id: &str,
    generation: u64,
) -> RuntimeResult<()> {
    if receipt.unit_id != unit_id || receipt.generation != generation {
        return Err(RuntimeError::RequestConflict {
            request_id: receipt.request_id.clone(),
        });
    }
    Ok(())
}

fn ensure_current_generation(
    record: &RuntimeUnitRecord,
    unit_id: &str,
    requested: u64,
) -> RuntimeResult<()> {
    if requested < record.spec.generation {
        return Err(RuntimeError::StaleGeneration {
            unit_id: unit_id.into(),
            requested,
            current: record.spec.generation,
        });
    }
    if requested != record.spec.generation {
        return Err(RuntimeError::GenerationConflict {
            unit_id: unit_id.into(),
            generation: requested,
        });
    }
    Ok(())
}

fn reconcile_completed_observation(
    record: &mut RuntimeUnitRecord,
    receipt: &RuntimeRequestReceipt,
) -> RuntimeResult<bool> {
    let Some(observation) = &receipt.observation else {
        return Ok(false);
    };
    if record.removed_at_ms.is_some()
        || record.spec.generation != receipt.generation
        || record.spec.unit_id != receipt.unit_id
        || record.observation == *observation
        || record.observation.observed_at_ms > observation.observed_at_ms
    {
        return Ok(false);
    }
    if validate_transition(&record.observation, observation, &record.spec).is_ok() {
        record.observation = observation.clone();
        record.validate().map_err(RuntimeError::Protocol)?;
        return Ok(true);
    }
    Ok(false)
}

fn reconcile_completed_removal(
    record: &mut RuntimeUnitRecord,
    receipt: &RuntimeRequestReceipt,
) -> RuntimeResult<bool> {
    let Some(removal) = &receipt.removal else {
        return Ok(false);
    };
    if record.spec.unit_id == receipt.unit_id
        && record.spec.generation == receipt.generation
        && record.removed_at_ms.is_none()
    {
        record.removed_at_ms = Some(removal.removed_at_ms);
        record.validate().map_err(RuntimeError::Protocol)?;
        return Ok(true);
    }
    Ok(false)
}

fn reconcile_completed_exec(
    record: &mut RuntimeUnitRecord,
    receipt: &RuntimeRequestReceipt,
) -> RuntimeResult<bool> {
    let Some(result) = &receipt.exec_result else {
        return Ok(false);
    };
    if record.removed_at_ms.is_some()
        || record.spec.generation != receipt.generation
        || record.spec.unit_id != receipt.unit_id
        || record.observation == result.observation
        || record.observation.observed_at_ms > result.observation.observed_at_ms
    {
        return Ok(false);
    }
    if validate_transition(&record.observation, &result.observation, &record.spec).is_ok() {
        record.observation = result.observation.clone();
        record.validate().map_err(RuntimeError::Protocol)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn validate_transition(
    current: &RuntimeObservation,
    next: &RuntimeObservation,
    spec: &crate::contract::RuntimeUnitSpec,
) -> RuntimeResult<()> {
    current
        .validate_against(spec)
        .map_err(RuntimeError::Protocol)?;
    next.validate_against(spec)
        .map_err(RuntimeError::Protocol)?;
    if current.state != RuntimeUnitState::Unknown
        && current.provider_resource_id.is_some()
        && current.provider_resource_id != next.provider_resource_id
    {
        return Err(RuntimeError::Protocol(
            "Runtime update changes provider resource identity".into(),
        ));
    }
    if next.observed_at_ms < current.observed_at_ms {
        return Err(RuntimeError::Protocol(
            "Runtime observation time moved backwards".into(),
        ));
    }
    if current.state.is_terminal() {
        if current != next {
            return Err(RuntimeError::Protocol(
                "terminal Runtime observation is immutable".into(),
            ));
        }
        return Ok(());
    }
    let allowed = current.state == next.state
        || matches!(
            (current.state, next.state),
            (RuntimeUnitState::Accepted, RuntimeUnitState::Preparing)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Starting)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Running)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Accepted, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Starting)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Running)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Preparing, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Running)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Starting, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Running, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Running, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Running, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Running, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Running, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Stopping, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Stopping, RuntimeUnitState::Failed)
                | (RuntimeUnitState::Stopping, RuntimeUnitState::Unknown)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Preparing)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Starting)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Running)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Stopping)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Stopped)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Succeeded)
                | (RuntimeUnitState::Unknown, RuntimeUnitState::Failed)
        );
    if !allowed {
        return Err(RuntimeError::Protocol(format!(
            "invalid Runtime transition {:?} -> {:?}",
            current.state, next.state
        )));
    }
    Ok(())
}

struct StateLock(File);

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

struct FileOperationLease(File);

impl RuntimeOperationLease for FileOperationLease {}

impl Drop for FileOperationLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn storage_key(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn validate_unit_id(unit_id: &str) -> RuntimeResult<()> {
    crate::contract::validate_id("unit_id", unit_id, 512).map_err(RuntimeError::InvalidRequest)
}

fn validate_request_id(request_id: &str) -> RuntimeResult<()> {
    crate::contract::validate_id("request_id", request_id, 512)
        .map_err(RuntimeError::InvalidRequest)
}

fn task_error(error: tokio::task::JoinError) -> RuntimeError {
    RuntimeError::Transport(format!("Runtime state task failed: {error}"))
}

#[cfg(test)]
mod tests;
