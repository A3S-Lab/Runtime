use crate::contract::RuntimeLogDiscontinuityReason;
use thiserror::Error;

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("invalid Runtime request: {0}")]
    InvalidRequest(String),
    #[error("Runtime unit {unit_id:?} was not found")]
    NotFound { unit_id: String },
    #[error("Runtime request {request_id:?} conflicts with a prior request")]
    RequestConflict { request_id: String },
    #[error("Runtime request {request_id:?} for unit {unit_id:?} was not found")]
    RequestNotFound { unit_id: String, request_id: String },
    #[error(
        "Runtime unit {unit_id:?} rejected stale generation {requested}; current generation is {current}"
    )]
    StaleGeneration {
        unit_id: String,
        requested: u64,
        current: u64,
    },
    #[error("Runtime unit {unit_id:?} has conflicting content for generation {generation}")]
    GenerationConflict { unit_id: String, generation: u64 },
    #[error("Runtime request deadline has expired: {0}")]
    DeadlineExceeded(String),
    #[error("Runtime provider lacks required capabilities: {0:?}")]
    UnsupportedCapabilities(Vec<String>),
    #[error("Runtime provider is unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("Runtime transport failed: {0}")]
    Transport(String),
    #[error(
        "Runtime log source for unit {unit_id:?} generation {generation} is discontinuous: {reason:?}"
    )]
    LogDiscontinuity {
        unit_id: String,
        generation: u64,
        cursor: Option<String>,
        reason: RuntimeLogDiscontinuityReason,
    },
    #[error("Runtime protocol failed: {0}")]
    Protocol(String),
}
