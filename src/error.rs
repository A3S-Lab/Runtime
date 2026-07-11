use thiserror::Error;

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("invalid runtime request: {0}")]
    InvalidRequest(String),
    #[error("runtime operation {operation_id:?} was not found")]
    NotFound { operation_id: String },
    #[error("runtime operation {operation_id:?} conflicts with an existing request")]
    OperationConflict { operation_id: String },
    #[error("runtime provider is unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("runtime transport failed: {0}")]
    Transport(String),
    #[error("runtime protocol failed: {0}")]
    Protocol(String),
}
