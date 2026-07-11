//! Provider-neutral execution contract and client for A3S runtimes.

mod client;
mod driver;
mod error;
mod managed;
mod operation;
mod registry;
mod selection;

pub mod contract;

pub use client::A3sRuntimeClient;
pub use driver::RuntimeDriver;
pub use error::{RuntimeError, RuntimeResult};
pub use managed::ManagedRuntimeClient;
pub use operation::{FileOperationStore, OperationRecord, OperationReservation, OperationStore};
pub use registry::{RuntimeClientRegistry, RuntimeProviderFactory};
pub use selection::{
    OperatorRuntimeConfig, ProviderId, RuntimeSelection, SelectionSource, SessionRuntimePolicy,
};
