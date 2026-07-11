use crate::{A3sRuntimeClient, ProviderId, RuntimeError, RuntimeResult, RuntimeSelection};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Typed construction boundary for one Runtime provider implementation.
///
/// Factories own provider-specific configuration and dependencies. Callers
/// select only a validated `ProviderId`; they never pass executable paths,
/// shell fragments, or provider-specific options into the shared client.
pub trait RuntimeProviderFactory: Send + Sync {
    fn provider_id(&self) -> &ProviderId;

    fn create(&self) -> RuntimeResult<Arc<dyn A3sRuntimeClient>>;
}

/// Registry used by control planes to resolve a selected provider without
/// provider-name branching.
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

    pub fn connect(
        &self,
        selection: &RuntimeSelection,
    ) -> RuntimeResult<Arc<dyn A3sRuntimeClient>> {
        self.factories
            .get(&selection.provider)
            .ok_or_else(|| {
                RuntimeError::ProviderUnavailable(format!(
                    "selected provider {:?} is not registered; explicit selections never fall back",
                    selection.provider.as_str()
                ))
            })?
            .create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{RuntimeCapabilities, RuntimeExecutionResult, RuntimeExecutionSpec};
    use crate::{OperatorRuntimeConfig, SelectionSource, SessionRuntimePolicy};
    use async_trait::async_trait;

    struct TestClient;

    #[async_trait]
    impl A3sRuntimeClient for TestClient {
        async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
            Err(RuntimeError::Protocol("not exercised".into()))
        }

        async fn submit(
            &self,
            _spec: &RuntimeExecutionSpec,
        ) -> RuntimeResult<RuntimeExecutionResult> {
            Err(RuntimeError::Protocol("not exercised".into()))
        }

        async fn inspect(&self, operation_id: &str) -> RuntimeResult<RuntimeExecutionResult> {
            Err(RuntimeError::NotFound {
                operation_id: operation_id.into(),
            })
        }

        async fn cancel(&self, operation_id: &str) -> RuntimeResult<RuntimeExecutionResult> {
            Err(RuntimeError::NotFound {
                operation_id: operation_id.into(),
            })
        }
    }

    struct TestFactory {
        provider: ProviderId,
    }

    impl RuntimeProviderFactory for TestFactory {
        fn provider_id(&self) -> &ProviderId {
            &self.provider
        }

        fn create(&self) -> RuntimeResult<Arc<dyn A3sRuntimeClient>> {
            Ok(Arc::new(TestClient))
        }
    }

    fn factory(provider: &str) -> Arc<dyn RuntimeProviderFactory> {
        Arc::new(TestFactory {
            provider: ProviderId::parse(provider).unwrap(),
        })
    }

    #[test]
    fn selected_factory_builds_the_shared_client() {
        let mut registry = RuntimeClientRegistry::new();
        registry.register(factory("docker")).unwrap();
        let selection = RuntimeSelection::resolve(
            &OperatorRuntimeConfig::default(),
            &SessionRuntimePolicy::default(),
        );
        assert!(registry.contains(&selection.provider));
        registry.connect(&selection).unwrap();
    }

    #[test]
    fn unavailable_explicit_provider_never_falls_back() {
        let mut registry = RuntimeClientRegistry::new();
        registry.register(factory("docker")).unwrap();
        let selection = RuntimeSelection {
            provider: ProviderId::parse("a3s-box").unwrap(),
            source: SelectionSource::OperatorConfig,
        };
        let error = registry.connect(&selection).err().unwrap();
        assert!(matches!(error, RuntimeError::ProviderUnavailable(_)));
    }

    #[test]
    fn duplicate_registration_is_rejected_without_replacement() {
        let mut registry = RuntimeClientRegistry::new();
        registry.register(factory("docker")).unwrap();
        assert!(registry.register(factory("docker")).is_err());
        assert!(registry
            .connect(&RuntimeSelection {
                provider: ProviderId::docker(),
                source: SelectionSource::SignedOutDefault,
            })
            .is_ok());
    }
}
