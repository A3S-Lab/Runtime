use super::{DockerArtifactResolver, DockerDriver};
use crate::contract::RuntimeCapabilities;
use crate::{
    A3sRuntimeClient, FileOperationStore, ManagedRuntimeClient, ProviderId, RuntimeProviderFactory,
    RuntimeResult,
};
use std::path::PathBuf;
use std::sync::Arc;

pub struct DockerProviderFactory {
    provider: ProviderId,
    executable: PathBuf,
    state_root: PathBuf,
    capabilities: RuntimeCapabilities,
    resolver: Arc<dyn DockerArtifactResolver>,
}

impl DockerProviderFactory {
    pub fn new(
        executable: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        capabilities: RuntimeCapabilities,
        resolver: Arc<dyn DockerArtifactResolver>,
    ) -> Self {
        Self {
            provider: ProviderId::docker(),
            executable: executable.into(),
            state_root: state_root.into(),
            capabilities,
            resolver,
        }
    }
}

impl RuntimeProviderFactory for DockerProviderFactory {
    fn provider_id(&self) -> &ProviderId {
        &self.provider
    }

    fn create(&self) -> RuntimeResult<Arc<dyn A3sRuntimeClient>> {
        self.capabilities
            .validate()
            .map_err(crate::RuntimeError::InvalidRequest)?;
        let driver = Arc::new(DockerDriver::new(
            self.executable.clone(),
            self.capabilities.clone(),
            Arc::clone(&self.resolver),
        ));
        Ok(Arc::new(ManagedRuntimeClient::new(
            Arc::new(FileOperationStore::new(&self.state_root)),
            driver,
        )))
    }
}
