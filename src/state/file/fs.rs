use crate::state::{RuntimeRequestReceipt, RuntimeUnitRecord};
use crate::{RuntimeError, RuntimeResult};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const MAX_RECORD_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RECEIPT_BYTES: u64 = 40 * 1024 * 1024;

pub(super) fn ensure_directory(path: &Path) -> RuntimeResult<()> {
    if path_exists(path)? {
        let metadata = std::fs::symlink_metadata(path).map_err(io_error("inspect state path"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(RuntimeError::Protocol(format!(
                "Runtime state path {} is not a real directory",
                path.display()
            )));
        }
        verify_owner(&metadata, path, "state directory")?;
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

pub(super) fn owner_only_open(path: &Path, label: &str) -> RuntimeResult<File> {
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
    let metadata = file.metadata().map_err(io_error("inspect state lock"))?;
    verify_owner_only_file(&metadata, path, label)?;
    Ok(file)
}

pub(super) fn read_required_record(path: &Path, unit_id: &str) -> RuntimeResult<RuntimeUnitRecord> {
    let record = read_optional_record(path)?.ok_or_else(|| RuntimeError::NotFound {
        unit_id: unit_id.into(),
    })?;
    if record.spec.unit_id != unit_id {
        return Err(RuntimeError::Protocol("Runtime state key mismatch".into()));
    }
    Ok(record)
}

pub(super) fn read_optional_record(path: &Path) -> RuntimeResult<Option<RuntimeUnitRecord>> {
    read_optional_json(path, MAX_RECORD_BYTES, "state record")
}

pub(super) fn read_required_receipt(
    path: &Path,
    unit_id: &str,
    request_id: &str,
) -> RuntimeResult<RuntimeRequestReceipt> {
    let receipt = read_optional_receipt(path)?.ok_or_else(|| RuntimeError::RequestNotFound {
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

pub(super) fn read_optional_receipt(path: &Path) -> RuntimeResult<Option<RuntimeRequestReceipt>> {
    let receipt: Option<RuntimeRequestReceipt> =
        read_optional_json(path, MAX_RECEIPT_BYTES, "request receipt")?;
    if let Some(receipt) = &receipt {
        receipt.validate().map_err(RuntimeError::Protocol)?;
    }
    Ok(receipt)
}

fn read_optional_json<T: DeserializeOwned>(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> RuntimeResult<Option<T>> {
    if !regular_file_exists(path, label)? {
        return Ok(None);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(io_error("inspect state file"))?;
    verify_owner_only_file(&metadata, path, label)?;
    if metadata.len() > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "Runtime {label} exceeds its size limit"
        )));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| {
        RuntimeError::Protocol(format!("Runtime {label} size cannot be represented"))
    })?;
    let mut bytes = Vec::with_capacity(capacity);
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
        .map_err(io_error("read state file"))?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| RuntimeError::Protocol(format!("invalid {label}: {error}")))?;
    Ok(Some(value))
}

fn regular_file_exists(path: &Path, label: &str) -> RuntimeResult<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(RuntimeError::Protocol(format!(
                "Runtime {label} {} is not a regular file",
                path.display()
            )))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("inspect state file")(error)),
    }
}

pub(super) fn path_exists(path: &Path) -> RuntimeResult<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("inspect state path")(error)),
    }
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

#[cfg(unix)]
fn verify_owner(metadata: &std::fs::Metadata, path: &Path, label: &str) -> RuntimeResult<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(RuntimeError::Protocol(format!(
            "Runtime {label} {} is owned by another user",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_owner(_metadata: &std::fs::Metadata, _path: &Path, _label: &str) -> RuntimeResult<()> {
    Ok(())
}

fn verify_owner_only_file(
    metadata: &std::fs::Metadata,
    path: &Path,
    label: &str,
) -> RuntimeResult<()> {
    verify_owner(metadata, path, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.permissions().mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err(RuntimeError::Protocol(format!(
                "Runtime {label} {} is not an owner-only unlinked file",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn atomic_write<T: Serialize>(path: &Path, value: &T, label: &str) -> RuntimeResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::Protocol(format!("{label} has no parent")))?;
    let bytes = serde_json::to_vec(value)
        .map_err(|error| RuntimeError::Protocol(format!("encode {label}: {error}")))?;

    cleanup_stale_staging_files(parent)?;
    #[cfg(test)]
    super::tests::inject_io_fault("state.atomic-write.before-create")
        .map_err(io_error("create state staging file"))?;

    let target_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RuntimeError::Protocol(format!("{label} has an invalid file name")))?;
    let staging_prefix = format!(".a3s-runtime-{target_name}-");
    let mut temporary = tempfile::Builder::new()
        .prefix(&staging_prefix)
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(io_error("create state staging file"))?;
    #[cfg(test)]
    test_failpoint("state.atomic-write.after-create");

    let split = bytes.len().div_ceil(2);
    temporary
        .write_all(&bytes[..split])
        .map_err(io_error("write state file"))?;
    #[cfg(test)]
    test_failpoint("state.atomic-write.after-partial-write");
    temporary
        .write_all(&bytes[split..])
        .map_err(io_error("write state file"))?;
    #[cfg(test)]
    test_failpoint("state.atomic-write.after-complete-write");
    temporary
        .as_file()
        .sync_all()
        .map_err(io_error("sync state file"))?;
    #[cfg(test)]
    test_failpoint("state.atomic-write.after-file-sync");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(io_error("secure state file"))?;
    }
    #[cfg(test)]
    test_failpoint("state.atomic-write.after-permissions");
    temporary
        .as_file()
        .sync_all()
        .map_err(io_error("sync secured state file"))?;
    temporary
        .persist(path)
        .map_err(|error| io_error("publish state file")(error.error))?;
    #[cfg(test)]
    test_failpoint("state.atomic-write.after-publish");
    sync_directory(parent)?;
    #[cfg(test)]
    test_failpoint("state.atomic-write.after-directory-sync");
    Ok(())
}

fn cleanup_stale_staging_files(parent: &Path) -> RuntimeResult<()> {
    let mut removed = false;
    for entry in std::fs::read_dir(parent).map_err(io_error("scan state staging files"))? {
        let entry = entry.map_err(io_error("scan state staging file"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(".tmp")
            || name.starts_with(".a3s-runtime-") && name.ends_with(".tmp"))
        {
            continue;
        }

        let path = entry.path();
        let metadata =
            std::fs::symlink_metadata(&path).map_err(io_error("inspect state staging file"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(RuntimeError::Protocol(format!(
                "Runtime state staging file {} is not a regular file",
                path.display()
            )));
        }
        verify_owner_only_file(&metadata, &path, "state staging file")?;
        std::fs::remove_file(&path).map_err(io_error("remove stale state staging file"))?;
        removed = true;
    }
    if removed {
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> RuntimeResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error("sync state directory"))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> RuntimeResult<()> {
    Ok(())
}

pub(super) fn io_error(action: &'static str) -> impl FnOnce(std::io::Error) -> RuntimeError {
    move |error| RuntimeError::Transport(format!("could not {action}: {error}"))
}

#[cfg(test)]
pub(super) fn test_failpoint(name: &str) {
    super::tests::hit_failpoint(name);
}
