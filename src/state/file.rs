use super::{
    RuntimeActionKind, RuntimeOperationLease, RuntimeRequestKind, RuntimeRequestReceipt,
    RuntimeRequestState, RuntimeStateReservation, RuntimeStateStore, RuntimeUnitRecord,
};
use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeObservation, RuntimeRemoval, RuntimeUnitState,
};
use crate::{RuntimeError, RuntimeResult};
use async_trait::async_trait;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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
        let path = self.record_path(&request.spec.unit_id)?;
        let mut record = if path.exists() {
            read_record(&path)?
        } else {
            let record =
                RuntimeUnitRecord::new(&request, now_ms).map_err(RuntimeError::Protocol)?;
            atomic_write(&path, &record)?;
            return reservation(record, &request.request_id);
        };

        if let Some(existing) = record.receipt(&request.request_id) {
            let digest = request.digest().map_err(RuntimeError::InvalidRequest)?;
            ensure_same_request(existing, RuntimeRequestKind::Apply, &digest)?;
            return reservation(record, &request.request_id);
        }

        let current_generation = record.spec.generation;
        if request.spec.generation < current_generation {
            return Err(RuntimeError::StaleGeneration {
                unit_id: request.spec.unit_id,
                requested: request.spec.generation,
                current: current_generation,
            });
        }

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
            let mut receipt =
                RuntimeRequestReceipt::pending_apply(&request).map_err(RuntimeError::Protocol)?;
            if !matches!(
                record.observation.state,
                RuntimeUnitState::Accepted | RuntimeUnitState::Unknown
            ) {
                receipt.complete_with_observation(record.observation.clone());
            }
            record.requests.insert(request.request_id.clone(), receipt);
        } else {
            record.spec = request.spec.clone();
            record.observation = RuntimeObservation::accepted(&request.spec, now_ms)
                .map_err(RuntimeError::Protocol)?;
            record.removed_at_ms = None;
            record.requests.insert(
                request.request_id.clone(),
                RuntimeRequestReceipt::pending_apply(&request).map_err(RuntimeError::Protocol)?,
            );
        }
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&path, &record)?;
        reservation(record, &request.request_id)
    }

    fn reserve_action_sync(
        &self,
        kind: RuntimeActionKind,
        request: RuntimeActionRequest,
        now_ms: u64,
    ) -> RuntimeResult<RuntimeStateReservation> {
        request.validate().map_err(RuntimeError::InvalidRequest)?;
        let _lock = self.lock(&request.unit_id)?;
        let path = self.record_path(&request.unit_id)?;
        if !path.is_file() {
            return Err(RuntimeError::NotFound {
                unit_id: request.unit_id,
            });
        }
        let mut record = read_record(&path)?;
        if let Some(existing) = record.receipt(&request.request_id) {
            let digest = request.digest().map_err(RuntimeError::InvalidRequest)?;
            ensure_same_request(existing, kind.into(), &digest)?;
            return reservation(record, &request.request_id);
        }
        if request.generation < record.spec.generation {
            return Err(RuntimeError::StaleGeneration {
                unit_id: request.unit_id,
                requested: request.generation,
                current: record.spec.generation,
            });
        }
        if request.generation != record.spec.generation {
            return Err(RuntimeError::GenerationConflict {
                unit_id: request.unit_id,
                generation: request.generation,
            });
        }

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
        record.requests.insert(request.request_id.clone(), receipt);
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&path, &record)?;
        reservation(record, &request.request_id)
    }

    fn load_sync(&self, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord> {
        validate_unit_id(unit_id)?;
        let _lock = self.lock(unit_id)?;
        let path = self.record_path(unit_id)?;
        if !path.is_file() {
            return Err(RuntimeError::NotFound {
                unit_id: unit_id.into(),
            });
        }
        let record = read_record(&path)?;
        if record.spec.unit_id != unit_id {
            return Err(RuntimeError::Protocol("Runtime state key mismatch".into()));
        }
        Ok(record)
    }

    fn update_observation_sync(
        &self,
        request_id: Option<String>,
        observation: RuntimeObservation,
    ) -> RuntimeResult<RuntimeUnitRecord> {
        observation.validate().map_err(RuntimeError::Protocol)?;
        let _lock = self.lock(&observation.unit_id)?;
        let path = self.record_path(&observation.unit_id)?;
        if !path.is_file() {
            return Err(RuntimeError::NotFound {
                unit_id: observation.unit_id,
            });
        }
        let mut record = read_record(&path)?;
        if record.removed_at_ms.is_some() {
            return Err(RuntimeError::Protocol(
                "cannot update an explicitly removed Runtime unit".into(),
            ));
        }
        validate_transition(&record.observation, &observation, &record.spec)?;
        record.observation = observation.clone();
        if let Some(request_id) = request_id {
            let receipt = record
                .receipt_mut(&request_id)
                .map_err(RuntimeError::Protocol)?;
            if receipt.kind == RuntimeRequestKind::Remove {
                return Err(RuntimeError::Protocol(
                    "remove request cannot complete with an observation".into(),
                ));
            }
            receipt.complete_with_observation(observation);
        }
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&path, &record)?;
        Ok(record)
    }

    fn complete_removal_sync(&self, removal: RuntimeRemoval) -> RuntimeResult<RuntimeUnitRecord> {
        removal.validate().map_err(RuntimeError::Protocol)?;
        let _lock = self.lock(&removal.unit_id)?;
        let path = self.record_path(&removal.unit_id)?;
        if !path.is_file() {
            return Err(RuntimeError::NotFound {
                unit_id: removal.unit_id,
            });
        }
        let mut record = read_record(&path)?;
        if removal.generation != record.spec.generation {
            return Err(RuntimeError::Protocol(
                "Runtime removal generation does not match stored unit".into(),
            ));
        }
        let receipt = record
            .receipt_mut(&removal.request_id)
            .map_err(RuntimeError::Protocol)?;
        if receipt.kind != RuntimeRequestKind::Remove {
            return Err(RuntimeError::Protocol(
                "non-remove request completed with removal result".into(),
            ));
        }
        receipt.complete_with_removal(removal.clone());
        record.removed_at_ms = Some(removal.removed_at_ms);
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&path, &record)?;
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

    fn record_path(&self, unit_id: &str) -> RuntimeResult<PathBuf> {
        validate_unit_id(unit_id)?;
        let records = self.root.join("units");
        ensure_directory(&records)?;
        Ok(records.join(format!("{}.json", storage_key(unit_id))))
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

    async fn load(&self, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord> {
        let store = self.clone();
        let unit_id = unit_id.to_owned();
        tokio::task::spawn_blocking(move || store.load_sync(&unit_id))
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
}

fn reservation(
    record: RuntimeUnitRecord,
    request_id: &str,
) -> RuntimeResult<RuntimeStateReservation> {
    let receipt = record
        .receipt(request_id)
        .cloned()
        .ok_or_else(|| RuntimeError::Protocol("reserved request receipt is missing".into()))?;
    Ok(RuntimeStateReservation {
        dispatch: receipt.state == RuntimeRequestState::Pending,
        record,
        receipt,
    })
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
        || current.state == RuntimeUnitState::Unknown
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

fn storage_key(unit_id: &str) -> String {
    format!("{:x}", Sha256::digest(unit_id.as_bytes()))
}

fn validate_unit_id(unit_id: &str) -> RuntimeResult<()> {
    crate::contract::validate_id("unit_id", unit_id, 512).map_err(RuntimeError::InvalidRequest)
}

fn ensure_directory(path: &Path) -> RuntimeResult<()> {
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path).map_err(io_error("inspect state path"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(RuntimeError::Protocol(format!(
                "Runtime state path {} is not a real directory",
                path.display()
            )));
        }
    } else {
        std::fs::create_dir_all(path).map_err(io_error("create state directory"))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(io_error("secure state directory"))?;
    }
    Ok(())
}

fn owner_only_open(path: &Path, label: &str) -> RuntimeResult<File> {
    reject_symlink(path, label)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(io_error("open state lock"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(io_error("secure state lock"))?;
    }
    Ok(file)
}

fn read_record(path: &Path) -> RuntimeResult<RuntimeUnitRecord> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error("inspect state record"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RuntimeError::Protocol(
            "Runtime state record is not a regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(io_error("read state record"))?;
    let record: RuntimeUnitRecord = serde_json::from_slice(&bytes)
        .map_err(|error| RuntimeError::Protocol(format!("invalid state record: {error}")))?;
    record.validate().map_err(RuntimeError::Protocol)?;
    Ok(record)
}

fn reject_symlink(path: &Path, label: &str) -> RuntimeResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{label} {} must not be a symbolic link",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect state file")(error)),
    }
}

fn atomic_write(path: &Path, record: &RuntimeUnitRecord) -> RuntimeResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::Protocol("state record has no parent".into()))?;
    let bytes = serde_json::to_vec(record)
        .map_err(|error| RuntimeError::Protocol(format!("encode state record: {error}")))?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(io_error("create state staging file"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(io_error("write state record"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(io_error("secure state record"))?;
    }
    temporary
        .persist(path)
        .map_err(|error| io_error("publish state record")(error.error))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error("sync state directory"))?;
    Ok(())
}

fn task_error(error: tokio::task::JoinError) -> RuntimeError {
    RuntimeError::Transport(format!("Runtime state task failed: {error}"))
}

fn io_error(action: &'static str) -> impl FnOnce(std::io::Error) -> RuntimeError {
    move |error| RuntimeError::Transport(format!("could not {action}: {error}"))
}
