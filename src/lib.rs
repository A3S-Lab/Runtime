//! Provider-neutral execution contract and client for A3S runtimes.

mod client;
mod error;
mod operation;
mod registry;
mod selection;

pub mod contract;

pub use client::A3sRuntimeClient;
pub use error::{RuntimeError, RuntimeResult};
pub use operation::{FileOperationStore, OperationRecord, OperationReservation, OperationStore};
pub use registry::{RuntimeClientRegistry, RuntimeProviderFactory};
pub use selection::{
    OperatorRuntimeConfig, ProviderId, RuntimeSelection, SelectionSource, SessionRuntimePolicy,
};
