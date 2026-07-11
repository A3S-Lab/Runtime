use super::{OperationRecord, OperationReservation, OperationStore};
use crate::contract::{ExecutionState, RuntimeExecutionResult, RuntimeExecutionSpec};
use crate::{RuntimeError, RuntimeResult};
use async_trait::async_trait;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileOperationStore {
    root: PathBuf,
}

impl FileOperationStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn reserve_sync(&self, spec: RuntimeExecutionSpec) -> RuntimeResult<OperationReservation> {
        spec.validate().map_err(RuntimeError::InvalidRequest)?;
        let _lock = self.lock(&spec.operation_id)?;
        let path = self.record_path(&spec.operation_id)?;
        if path.exists() {
            let record = read_record(&path)?;
            if record.spec.digest().map_err(RuntimeError::Protocol)?
                != spec.digest().map_err(RuntimeError::InvalidRequest)?
            {
                return Err(RuntimeError::OperationConflict {
                    operation_id: spec.operation_id,
                });
            }
            return Ok(OperationReservation {
                created: false,
                record,
            });
        }
        let record = OperationRecord::queued(spec).map_err(RuntimeError::Protocol)?;
        atomic_write(&path, &record)?;
        Ok(OperationReservation {
            created: true,
            record,
        })
    }

    fn load_sync(&self, operation_id: &str) -> RuntimeResult<OperationRecord> {
        validate_operation_id(operation_id)?;
        let _lock = self.lock(operation_id)?;
        let path = self.record_path(operation_id)?;
        if !path.is_file() {
            return Err(RuntimeError::NotFound {
                operation_id: operation_id.into(),
            });
        }
        let record = read_record(&path)?;
        if record.spec.operation_id != operation_id {
            return Err(RuntimeError::Protocol(
                "operation store key mismatch".into(),
            ));
        }
        Ok(record)
    }

    fn update_sync(&self, result: RuntimeExecutionResult) -> RuntimeResult<OperationRecord> {
        result.validate().map_err(RuntimeError::Protocol)?;
        let _lock = self.lock(&result.operation_id)?;
        let path = self.record_path(&result.operation_id)?;
        if !path.is_file() {
            return Err(RuntimeError::NotFound {
                operation_id: result.operation_id,
            });
        }
        let mut record = read_record(&path)?;
        validate_transition(&record.result, &result)?;
        if record.result == result {
            return Ok(record);
        }
        record.result = result;
        record.validate().map_err(RuntimeError::Protocol)?;
        atomic_write(&path, &record)?;
        Ok(record)
    }

    fn lock(&self, operation_id: &str) -> RuntimeResult<OperationLock> {
        validate_operation_id(operation_id)?;
        ensure_directory(&self.root)?;
        let locks = self.root.join("locks");
        ensure_directory(&locks)?;
        let path = locks.join(format!("{}.lock", storage_key(operation_id)));
        let file = owner_only_open(&path)?;
        file.lock_exclusive().map_err(io_error("lock operation"))?;
        Ok(OperationLock(file))
    }

    fn record_path(&self, operation_id: &str) -> RuntimeResult<PathBuf> {
        validate_operation_id(operation_id)?;
        let records = self.root.join("records");
        ensure_directory(&records)?;
        Ok(records.join(format!("{}.json", storage_key(operation_id))))
    }
}

#[async_trait]
impl OperationStore for FileOperationStore {
    async fn reserve(&self, spec: &RuntimeExecutionSpec) -> RuntimeResult<OperationReservation> {
        let store = self.clone();
        let spec = spec.clone();
        tokio::task::spawn_blocking(move || store.reserve_sync(spec))
            .await
            .map_err(|error| RuntimeError::Transport(format!("operation task failed: {error}")))?
    }

    async fn load(&self, operation_id: &str) -> RuntimeResult<OperationRecord> {
        let store = self.clone();
        let operation_id = operation_id.to_owned();
        tokio::task::spawn_blocking(move || store.load_sync(&operation_id))
            .await
            .map_err(|error| RuntimeError::Transport(format!("operation task failed: {error}")))?
    }

    async fn update(&self, result: &RuntimeExecutionResult) -> RuntimeResult<OperationRecord> {
        let store = self.clone();
        let result = result.clone();
        tokio::task::spawn_blocking(move || store.update_sync(result))
            .await
            .map_err(|error| RuntimeError::Transport(format!("operation task failed: {error}")))?
    }
}

struct OperationLock(File);

impl Drop for OperationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

pub(crate) fn execution_id(operation_id: &str) -> String {
    format!("execution-{}", &storage_key(operation_id)[..32])
}

fn storage_key(operation_id: &str) -> String {
    format!("{:x}", Sha256::digest(operation_id.as_bytes()))
}

fn validate_operation_id(operation_id: &str) -> RuntimeResult<()> {
    if operation_id.is_empty() || operation_id.len() > 512 || operation_id.contains('\0') {
        return Err(RuntimeError::InvalidRequest(
            "operation_id must contain 1 to 512 non-NUL bytes".into(),
        ));
    }
    Ok(())
}

fn validate_transition(
    current: &RuntimeExecutionResult,
    next: &RuntimeExecutionResult,
) -> RuntimeResult<()> {
    if current.operation_id != next.operation_id
        || current.execution_id != next.execution_id
        || current.spec_digest != next.spec_digest
        || current.role != next.role
    {
        return Err(RuntimeError::Protocol(
            "operation update changes immutable identity".into(),
        ));
    }
    let allowed = current == next
        || matches!(
            (current.state, next.state),
            (ExecutionState::Queued, ExecutionState::Running)
                | (ExecutionState::Queued, ExecutionState::Failed)
                | (ExecutionState::Queued, ExecutionState::Cancelled)
                | (ExecutionState::Running, ExecutionState::Succeeded)
                | (ExecutionState::Running, ExecutionState::Failed)
                | (ExecutionState::Running, ExecutionState::Cancelled)
        );
    if !allowed {
        return Err(RuntimeError::Protocol(format!(
            "invalid operation transition {:?} -> {:?}",
            current.state, next.state
        )));
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> RuntimeResult<()> {
    if path.exists() {
        let metadata =
            std::fs::symlink_metadata(path).map_err(io_error("inspect state directory"))?;
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

fn owner_only_open(path: &Path) -> RuntimeResult<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(io_error("open operation lock"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(io_error("secure operation lock"))?;
    }
    Ok(file)
}

fn read_record(path: &Path) -> RuntimeResult<OperationRecord> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error("inspect operation record"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RuntimeError::Protocol(
            "operation record is not a regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(io_error("read operation record"))?;
    let record: OperationRecord = serde_json::from_slice(&bytes)
        .map_err(|error| RuntimeError::Protocol(format!("invalid operation record: {error}")))?;
    record.validate().map_err(RuntimeError::Protocol)?;
    Ok(record)
}

fn atomic_write(path: &Path, record: &OperationRecord) -> RuntimeResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::Protocol("operation record has no parent".into()))?;
    let bytes = serde_json::to_vec(record)
        .map_err(|error| RuntimeError::Protocol(format!("encode operation record: {error}")))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(io_error("create operation staging file"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(io_error("write operation record"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(io_error("secure operation record"))?;
    }
    temporary
        .persist(path)
        .map_err(|error| io_error("publish operation record")(error.error))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error("sync operation directory"))?;
    Ok(())
}

fn io_error(action: &'static str) -> impl FnOnce(std::io::Error) -> RuntimeError {
    move |error| RuntimeError::Transport(format!("could not {action}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        ArtifactRef, NetworkPolicy, ResourceLimits, RuntimeRole, SubmissionPolicy,
    };
    use std::sync::Arc;

    fn artifact() -> ArtifactRef {
        ArtifactRef {
            digest: format!("sha256:{}", "a".repeat(64)),
            media_type: "application/vnd.a3s.asset.v1".into(),
        }
    }

    fn spec(operation_id: &str) -> RuntimeExecutionSpec {
        RuntimeExecutionSpec {
            schema: RuntimeExecutionSpec::SCHEMA.into(),
            operation_id: operation_id.into(),
            role: RuntimeRole::Candidate,
            asset: artifact(),
            work_image: artifact(),
            protected_mounts: vec![],
            protected_result_schema: None,
            submission_policy: Some(SubmissionPolicy {
                include: vec!["**".into()],
                exclude: vec![],
                max_files: 1,
                max_total_bytes: 1,
                max_file_bytes: 1,
            }),
            network: NetworkPolicy::None,
            resources: ResourceLimits {
                wall_time_ms: 1,
                cpu_millis: 1,
                memory_bytes: 1,
                scratch_bytes: 1,
                output_bytes: 1,
            },
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reservation_is_idempotent_and_conflicts_on_changed_spec() {
        let directory = tempfile::tempdir().unwrap();
        let store = FileOperationStore::new(directory.path());
        let first = store.reserve(&spec("run/candidate")).await.unwrap();
        assert!(first.created);
        let repeated = store.reserve(&spec("run/candidate")).await.unwrap();
        assert!(!repeated.created);
        assert_eq!(first.record, repeated.record);
        let mut changed = spec("run/candidate");
        changed.resources.memory_bytes = 2;
        assert!(matches!(
            store.reserve(&changed).await,
            Err(RuntimeError::OperationConflict { .. })
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_reservations_publish_one_complete_record() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(FileOperationStore::new(directory.path()));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let store = Arc::clone(&store);
            tasks.push(tokio::spawn(async move {
                store.reserve(&spec("run/concurrent")).await.unwrap()
            }));
        }
        let mut created = 0;
        for task in tasks {
            created += usize::from(task.await.unwrap().created);
        }
        assert_eq!(created, 1);
        store
            .load("run/concurrent")
            .await
            .unwrap()
            .validate()
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn state_transitions_are_monotonic_and_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let store = FileOperationStore::new(directory.path());
        let queued = store
            .reserve(&spec("run/transitions"))
            .await
            .unwrap()
            .record
            .result;
        let mut running = queued.clone();
        running.state = ExecutionState::Running;
        running.started_at_ms = Some(1);
        store.update(&running).await.unwrap();
        assert_eq!(store.update(&running).await.unwrap().result, running);
        assert!(store.update(&queued).await.is_err());
        let mut wrong_identity = running;
        wrong_identity.execution_id = "substituted".into();
        assert!(store.update(&wrong_identity).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn symlink_state_root_is_rejected() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let root = directory.path().join("state");
        symlink(target.path(), &root).unwrap();
        assert!(FileOperationStore::new(root)
            .reserve(&spec("run/symlink"))
            .await
            .is_err());
    }
}
