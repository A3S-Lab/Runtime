use a3s_runtime::contract::{
    IsolationLevel, NetworkMode, ResourceControl, RuntimeActionRequest, RuntimeCapabilities,
    RuntimeExecRequest, RuntimeExecResult, RuntimeFeature, RuntimeInspection, RuntimeLogChunk,
    RuntimeLogQuery, RuntimeObservation, RuntimeRemoval, RuntimeUnitClass, RuntimeUnitSpec,
    RuntimeUnitState,
};
use a3s_runtime::{ProviderId, RuntimeDriver, RuntimeError, RuntimeResult, RuntimeUnitRecord};
use async_trait::async_trait;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const IMAGE_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const PROVIDER_BUILD: &str = "process-race-driver/1";
const FAILPOINT_ENV: &str = "A3S_RUNTIME_PROCESS_DRIVER_FAILPOINT";
const FAILPOINT_READY_ENV: &str = "A3S_RUNTIME_PROCESS_DRIVER_FAILPOINT_READY";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderResource {
    pub resource_id: String,
    pub unit_id: String,
    pub generation: u64,
    pub state: RuntimeUnitState,
    pub observed_at_ms: u64,
    pub started_at_ms: u64,
    pub apply_dispatches: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessRaceDriver {
    root: PathBuf,
    provider_id: ProviderId,
}

impl ProcessRaceDriver {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            provider_id: ProviderId::parse("process-race-runtime")
                .expect("valid process-race provider ID"),
        }
    }

    pub fn inventory(&self, unit_id: &str) -> RuntimeResult<Vec<ProviderResource>> {
        self.with_lock(|root| inventory_unlocked(root, unit_id))
    }

    pub fn inject_duplicate(&self, unit_id: &str, generation: u64) -> RuntimeResult<()> {
        self.with_lock(|root| {
            let resources = inventory_unlocked(root, unit_id)?;
            let source = resources
                .iter()
                .find(|resource| resource.generation == generation)
                .ok_or_else(|| RuntimeError::Protocol("duplicate source is absent".into()))?;
            let mut duplicate = source.clone();
            duplicate.resource_id = format!("{}/duplicate", source.resource_id);
            write_resource(root, &duplicate)
        })
    }

    fn with_lock<T>(&self, operation: impl FnOnce(&Path) -> RuntimeResult<T>) -> RuntimeResult<T> {
        ensure_provider_root(&self.root)?;
        let lock_path = self.root.join("provider.lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(lock_path)
            .map_err(io_error("open process provider lock"))?;
        lock.lock_exclusive()
            .map_err(io_error("lock process provider"))?;
        let _guard = ProviderLock(lock);
        operation(&self.root)
    }

    fn apply_sync(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeObservation> {
        self.with_lock(|root| {
            let resources = inventory_unlocked(root, &spec.unit_id)?;
            if let Some(generation) = duplicate_generation(&resources) {
                return Err(duplicate_error(&spec.unit_id, generation));
            }
            let current_resources = resources
                .iter()
                .filter(|resource| resource.generation == spec.generation)
                .collect::<Vec<_>>();

            let mut resource = current_resources.first().map_or_else(
                || ProviderResource {
                    resource_id: format!("process/{}/g{}", spec.unit_id, spec.generation),
                    unit_id: spec.unit_id.clone(),
                    generation: spec.generation,
                    state: RuntimeUnitState::Running,
                    observed_at_ms: current.observed_at_ms.saturating_add(1),
                    started_at_ms: current.observed_at_ms.saturating_add(1),
                    apply_dispatches: 0,
                },
                |resource| (*resource).clone(),
            );
            resource.apply_dispatches = resource.apply_dispatches.saturating_add(1);
            resource.state = RuntimeUnitState::Running;
            resource.observed_at_ms = resource
                .observed_at_ms
                .max(current.observed_at_ms.saturating_add(1));
            write_resource(root, &resource)?;
            hit_failpoint("provider.apply.after-current-publish");

            for stale in resources
                .iter()
                .filter(|candidate| candidate.generation != spec.generation)
            {
                remove_resource(root, &stale.resource_id)?;
            }
            let final_inventory = inventory_unlocked(root, &spec.unit_id)?;
            if final_inventory.len() != 1
                || final_inventory[0].generation != spec.generation
                || final_inventory[0].resource_id != resource.resource_id
            {
                return Err(RuntimeError::Protocol(format!(
                    "process provider did not converge {:?} generation {} to one resource",
                    spec.unit_id, spec.generation
                )));
            }
            Ok(observation_from_resource(current, &resource))
        })
    }

    fn inspect_sync(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection> {
        self.with_lock(|root| {
            let resources = inventory_unlocked(root, &unit.spec.unit_id)?;
            let current = resources
                .iter()
                .filter(|resource| resource.generation == unit.spec.generation)
                .collect::<Vec<_>>();
            if current.len() > 1 {
                return Err(duplicate_error(&unit.spec.unit_id, unit.spec.generation));
            }
            let Some(resource) = current.first() else {
                return Ok(RuntimeInspection::NotFound {
                    schema: RuntimeInspection::SCHEMA.into(),
                    unit_id: unit.spec.unit_id.clone(),
                    last_generation: Some(unit.spec.generation),
                });
            };
            Ok(RuntimeInspection::Found {
                schema: RuntimeInspection::SCHEMA.into(),
                observation: Box::new(observation_from_resource(&unit.observation, resource)),
            })
        })
    }

    fn stop_sync(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeObservation> {
        self.with_lock(|root| {
            let resources = inventory_unlocked(root, &unit.spec.unit_id)?;
            let mut current = resources
                .into_iter()
                .filter(|resource| resource.generation == unit.spec.generation)
                .collect::<Vec<_>>();
            if current.len() > 1 {
                return Err(duplicate_error(&unit.spec.unit_id, unit.spec.generation));
            }
            let Some(mut resource) = current.pop() else {
                let mut unknown = unit.observation.clone();
                unknown.state = RuntimeUnitState::Unknown;
                unknown.observed_at_ms = unknown.observed_at_ms.saturating_add(1);
                unknown.finished_at_ms = None;
                unknown.health = None;
                unknown.outputs.clear();
                unknown.failure = None;
                return Ok(unknown);
            };
            resource.state = RuntimeUnitState::Stopped;
            resource.observed_at_ms = resource
                .observed_at_ms
                .max(unit.observation.observed_at_ms.saturating_add(1));
            write_resource(root, &resource)?;
            Ok(observation_from_resource(&unit.observation, &resource))
        })
    }

    fn remove_sync(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeRemoval> {
        self.with_lock(|root| {
            let resources = inventory_unlocked(root, &unit.spec.unit_id)?;
            let current = resources
                .iter()
                .filter(|resource| resource.generation == unit.spec.generation)
                .collect::<Vec<_>>();
            if current.len() > 1 {
                return Err(duplicate_error(&unit.spec.unit_id, unit.spec.generation));
            }
            let already_absent = current.is_empty();
            if let Some(resource) = current.first() {
                remove_resource(root, &resource.resource_id)?;
            }
            Ok(RuntimeRemoval {
                schema: RuntimeRemoval::SCHEMA.into(),
                request_id: request.request_id.clone(),
                unit_id: request.unit_id.clone(),
                generation: request.generation,
                removed_at_ms: now_ms(),
                already_absent,
            })
        })
    }
}

#[async_trait]
impl RuntimeDriver for ProcessRaceDriver {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        Ok(RuntimeCapabilities {
            schema: RuntimeCapabilities::SCHEMA.into(),
            provider_id: self.provider_id.clone(),
            provider_build: PROVIDER_BUILD.into(),
            unit_classes: vec![RuntimeUnitClass::Service],
            artifact_media_types: vec![IMAGE_MEDIA_TYPE.into()],
            isolation_levels: vec![IsolationLevel::Container],
            network_modes: vec![NetworkMode::None],
            mount_kinds: Vec::new(),
            health_check_kinds: Vec::new(),
            resource_controls: vec![
                ResourceControl::Cpu,
                ResourceControl::Memory,
                ResourceControl::Pids,
            ],
            features: vec![
                RuntimeFeature::DurableIdentity,
                RuntimeFeature::Stop,
                RuntimeFeature::Remove,
            ],
        })
    }

    async fn apply(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeObservation> {
        let driver = self.clone();
        let spec = spec.clone();
        let current = current.clone();
        tokio::task::spawn_blocking(move || driver.apply_sync(&spec, &current))
            .await
            .map_err(task_error)?
    }

    async fn inspect(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection> {
        let driver = self.clone();
        let unit = unit.clone();
        tokio::task::spawn_blocking(move || driver.inspect_sync(&unit))
            .await
            .map_err(task_error)?
    }

    async fn stop(
        &self,
        unit: &RuntimeUnitRecord,
        _request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeObservation> {
        let driver = self.clone();
        let unit = unit.clone();
        tokio::task::spawn_blocking(move || driver.stop_sync(&unit))
            .await
            .map_err(task_error)?
    }

    async fn remove(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeRemoval> {
        let driver = self.clone();
        let unit = unit.clone();
        let request = request.clone();
        tokio::task::spawn_blocking(move || driver.remove_sync(&unit, &request))
            .await
            .map_err(task_error)?
    }

    async fn logs(
        &self,
        _unit: &RuntimeUnitRecord,
        _query: &RuntimeLogQuery,
    ) -> RuntimeResult<Vec<RuntimeLogChunk>> {
        Err(RuntimeError::UnsupportedCapabilities(vec![
            "feature:Logs".into()
        ]))
    }

    async fn exec(
        &self,
        _unit: &RuntimeUnitRecord,
        _request: &RuntimeExecRequest,
    ) -> RuntimeResult<RuntimeExecResult> {
        Err(RuntimeError::UnsupportedCapabilities(vec![
            "feature:Exec".into()
        ]))
    }
}

fn ensure_provider_root(root: &Path) -> RuntimeResult<()> {
    std::fs::create_dir_all(root.join("resources"))
        .map_err(io_error("create process provider root"))?;
    Ok(())
}

fn inventory_unlocked(root: &Path, unit_id: &str) -> RuntimeResult<Vec<ProviderResource>> {
    let mut resources = Vec::new();
    for entry in std::fs::read_dir(root.join("resources"))
        .map_err(io_error("read process provider inventory"))?
    {
        let entry = entry.map_err(io_error("read process provider resource"))?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(io_error("read provider resource"))?;
        let resource: ProviderResource = serde_json::from_slice(&bytes).map_err(|error| {
            RuntimeError::Protocol(format!("invalid provider resource: {error}"))
        })?;
        if resource.unit_id == unit_id {
            resources.push(resource);
        }
    }
    resources.sort_by(|left, right| {
        (left.generation, &left.resource_id).cmp(&(right.generation, &right.resource_id))
    });
    Ok(resources)
}

fn write_resource(root: &Path, resource: &ProviderResource) -> RuntimeResult<()> {
    let path = resource_path(root, &resource.resource_id);
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(resource)
        .map_err(|error| RuntimeError::Protocol(format!("encode provider resource: {error}")))?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(io_error("create provider staging file"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(io_error("write provider resource"))?;
    std::fs::rename(&temporary, &path).map_err(io_error("publish provider resource"))?;
    sync_directory(&root.join("resources"))
}

fn remove_resource(root: &Path, resource_id: &str) -> RuntimeResult<()> {
    let path = resource_path(root, resource_id);
    match std::fs::remove_file(path) {
        Ok(()) => sync_directory(&root.join("resources")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove provider resource")(error)),
    }
}

fn resource_path(root: &Path, resource_id: &str) -> PathBuf {
    let key = format!("{:x}", Sha256::digest(resource_id.as_bytes()));
    root.join("resources").join(format!("{key}.json"))
}

fn observation_from_resource(
    current: &RuntimeObservation,
    resource: &ProviderResource,
) -> RuntimeObservation {
    let mut observation = current.clone();
    observation.state = resource.state;
    observation.provider_resource_id = Some(resource.resource_id.clone());
    observation.provider_build = Some(PROVIDER_BUILD.into());
    observation.observed_at_ms = resource.observed_at_ms.max(current.observed_at_ms);
    observation.started_at_ms = Some(resource.started_at_ms);
    observation.finished_at_ms = resource
        .state
        .is_terminal()
        .then_some(observation.observed_at_ms);
    observation.health = None;
    observation.outputs.clear();
    observation.failure = None;
    observation
}

fn duplicate_error(unit_id: &str, generation: u64) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "process provider found duplicate resources for {unit_id:?} generation {generation}"
    ))
}

fn duplicate_generation(resources: &[ProviderResource]) -> Option<u64> {
    resources
        .windows(2)
        .find_map(|pair| (pair[0].generation == pair[1].generation).then_some(pair[0].generation))
}

fn hit_failpoint(name: &str) {
    if !matches!(std::env::var(FAILPOINT_ENV), Ok(value) if value == name) {
        return;
    }
    let ready = std::env::var(FAILPOINT_READY_ENV).expect("provider failpoint ready path");
    std::fs::write(ready, name).expect("publish provider failpoint readiness");
    loop {
        std::thread::park_timeout(std::time::Duration::from_secs(60));
    }
}

fn now_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> RuntimeResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error("sync process provider directory"))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> RuntimeResult<()> {
    Ok(())
}

fn task_error(error: tokio::task::JoinError) -> RuntimeError {
    RuntimeError::Transport(format!("process provider task failed: {error}"))
}

fn io_error(action: &'static str) -> impl FnOnce(std::io::Error) -> RuntimeError {
    move |error| RuntimeError::Transport(format!("could not {action}: {error}"))
}

struct ProviderLock(File);

impl Drop for ProviderLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
