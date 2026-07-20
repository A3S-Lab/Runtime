mod artifact;
mod capabilities;
mod network;
mod observation;
mod process;
mod protocol;
mod resource;
mod unit;

pub use artifact::{ArtifactRef, RuntimeOutputArtifact};
pub use capabilities::{ResourceControl, RuntimeCapabilities, RuntimeFeature};
pub use network::{NetworkMode, RuntimeNetworkSpec, RuntimePort, TransportProtocol};
pub use observation::{
    RuntimeEvidence, RuntimeFailure, RuntimeHealthObservation, RuntimeHealthState,
    RuntimeInspection, RuntimeObservation, RuntimeUnitState, RuntimeUsage,
};
pub use process::{RuntimeProcessSpec, SecretReference, SecretTarget};
pub use protocol::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeExecRequest, RuntimeExecResult,
    RuntimeLogChunk, RuntimeLogDiscontinuityReason, RuntimeLogQuery, RuntimeLogStream,
    RuntimeRemoval,
};
pub use resource::{IsolationLevel, ResourceLimits};
pub use unit::{
    HealthCheckKind, HealthProbe, MountKind, RestartPolicy, RuntimeHealthCheck, RuntimeMount,
    RuntimeMountSource, RuntimeOutputSpec, RuntimeUnitClass, RuntimeUnitSpec,
};

pub(crate) fn validate_digest(value: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err("digest must use sha256".into());
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("digest must contain exactly 64 hexadecimal characters".into());
    }
    Ok(())
}

pub(crate) fn validate_nonempty(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max || value.contains('\0') || value.contains(['\r', '\n'])
    {
        return Err(format!(
            "{label} must be a bounded nonempty single-line value"
        ));
    }
    Ok(())
}

pub(crate) fn validate_id(label: &str, value: &str, max: usize) -> Result<(), String> {
    validate_nonempty(label, value, max)?;
    if value
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte)))
    {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(())
}

pub(crate) fn validate_name(label: &str, value: &str) -> Result<(), String> {
    validate_nonempty(label, value, 255)?;
    if value
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || b"-_ .".contains(&byte)))
        || value.starts_with(['-', '_', '.', ' '])
        || value.ends_with(['-', '_', '.', ' '])
    {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(())
}

pub(crate) fn validate_absolute_path(label: &str, value: &str) -> Result<(), String> {
    if !value.starts_with('/')
        || value.len() > 4096
        || value.contains('\0')
        || value.split('/').any(|segment| segment == "..")
    {
        return Err(format!(
            "{label} must be a bounded absolute path without '..'"
        ));
    }
    Ok(())
}

pub(crate) fn validate_uri(label: &str, value: &str) -> Result<(), String> {
    validate_nonempty(label, value, 4096)?;
    let Some((scheme, rest)) = value.split_once("://") else {
        return Err(format!("{label} must contain a URI scheme"));
    };
    if scheme.is_empty()
        || rest.is_empty()
        || !scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}
