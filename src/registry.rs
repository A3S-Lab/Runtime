use crate::{ProviderId, RuntimeClient, RuntimeError, RuntimeResult};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Typed construction boundary for one Runtime provider implementation.
pub trait RuntimeProviderFactory: Send + Sync {
    fn provider_id(&self) -> &ProviderId;

    fn create(&self) -> RuntimeResult<Arc<dyn RuntimeClient>>;
}

/// Registry of provider factories. Selection policy belongs to the caller;
/// this registry never falls back to a default provider.
#[derive(Default)]
pub struct RuntimeClientRegistry {
    factories: BTreeMap<ProviderId, Arc<dyn RuntimeProviderFactory>>,
}

impl RuntimeClientRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, factory: Arc<dyn RuntimeProviderFactory>) -> RuntimeResult<()> {
        let provider = factory.provider_id().clone();
        if self.factories.contains_key(&provider) {
            return Err(RuntimeError::InvalidRequest(format!(
                "Runtime provider {provider:?} is already registered"
            )));
        }
        self.factories.insert(provider, factory);
        Ok(())
    }

    pub fn contains(&self, provider: &ProviderId) -> bool {
        self.factories.contains_key(provider)
    }

    pub fn connect(&self, provider: &ProviderId) -> RuntimeResult<Arc<dyn RuntimeClient>> {
        self.factories
            .get(provider)
            .ok_or_else(|| {
                RuntimeError::ProviderUnavailable(format!(
                    "provider {:?} is not registered",
                    provider.as_str()
                ))
            })?
            .create()
    }
}
