//! General provider-neutral Task and Service Runtime contracts for A3S.

mod client;
mod clock;
mod conformance;
mod driver;
mod error;
mod managed;
mod provider;
mod registry;
mod state;

pub mod contract;

pub use client::RuntimeClient;
pub use clock::{RuntimeClock, SystemRuntimeClock};
pub use conformance::{
    required_runtime_profiles, runtime_profile_requirements, verify_runtime_base,
    verify_runtime_profiles, verify_runtime_provider, RuntimeBaseConformanceCase,
    RuntimeBaseConformanceReport, RuntimeConformanceCase, RuntimeConformanceFixture,
    RuntimeConformanceInventory, RuntimeConformanceProfile, RuntimeConformanceProfileEvidence,
    RuntimeConformanceProfileRequirements, RuntimeConformanceReport, RuntimeConformanceSuiteReport,
};
pub use driver::RuntimeDriver;
pub use error::{RuntimeError, RuntimeResult};
pub use managed::ManagedRuntimeClient;
pub use provider::ProviderId;
pub use registry::{RuntimeClientRegistry, RuntimeProviderFactory};
pub use state::{
    FileRuntimeStateStore, RuntimeActionKind, RuntimeOperationLease, RuntimeRequestKind,
    RuntimeRequestReceipt, RuntimeRequestState, RuntimeStateReservation, RuntimeStateStore,
    RuntimeUnitRecord,
};
