//! Provider-neutral execution contract and client for A3S runtimes.

mod client;
mod error;
mod selection;

pub mod contract;

pub use client::A3sRuntimeClient;
pub use error::{RuntimeError, RuntimeResult};
pub use selection::{
    OperatorRuntimeConfig, ProviderId, RuntimeSelection, SelectionSource, SessionRuntimePolicy,
};
