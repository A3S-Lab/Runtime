use a3s_runtime::contract::{
    IsolationLevel, NetworkMode, ResourceControl, RuntimeActionRequest, RuntimeCapabilities,
    RuntimeExecRequest, RuntimeExecResult, RuntimeFeature, RuntimeInspection, RuntimeLogChunk,
    RuntimeLogQuery, RuntimeLogStream, RuntimeObservation, RuntimeRemoval, RuntimeUnitClass,
    RuntimeUnitSpec, RuntimeUnitState,
};
use a3s_runtime::{ProviderId, RuntimeDriver, RuntimeError, RuntimeResult, RuntimeUnitRecord};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

pub const IMAGE_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FaultOperation {
    Capabilities,
    Apply,
    Inspect,
    Stop,
    Remove,
    Logs,
    Exec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultBoundary {
    BeforeEffect,
    AfterEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultMode {
    Transport,
    Hang,
    Panic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArmedFault {
    operation: FaultOperation,
    boundary: FaultBoundary,
    mode: FaultMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceEvent {
    pub operation: FaultOperation,
    pub boundary: FaultBoundary,
}

#[derive(Debug, Clone)]
struct ProviderResource {
    observation: RuntimeObservation,
}

pub struct DeterministicFaultDriver {
    provider_id: ProviderId,
    fault: Mutex<Option<ArmedFault>>,
    trace: Mutex<Vec<TraceEvent>>,
    resources: Mutex<BTreeMap<String, BTreeMap<u64, ProviderResource>>>,
    exec_results: Mutex<BTreeMap<String, RuntimeExecResult>>,
    effects: Mutex<BTreeMap<FaultOperation, usize>>,
}

impl Default for DeterministicFaultDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl DeterministicFaultDriver {
    pub fn new() -> Self {
        Self {
            provider_id: ProviderId::parse("deterministic-fault-runtime")
                .expect("valid deterministic provider ID"),
            fault: Mutex::new(None),
            trace: Mutex::new(Vec::new()),
            resources: Mutex::new(BTreeMap::new()),
            exec_results: Mutex::new(BTreeMap::new()),
            effects: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn arm(&self, operation: FaultOperation, boundary: FaultBoundary, mode: FaultMode) {
        let mut fault = self.fault.lock().expect("fault plan lock");
        assert!(fault.is_none(), "only one deterministic fault may be armed");
        *fault = Some(ArmedFault {
            operation,
            boundary,
            mode,
        });
    }

    pub fn trace(&self) -> Vec<TraceEvent> {
        self.trace.lock().expect("trace lock").clone()
    }

    pub fn resource_count(&self, unit_id: &str) -> usize {
        self.resources
            .lock()
            .expect("provider resource lock")
            .get(unit_id)
            .map(BTreeMap::len)
            .unwrap_or_default()
    }

    pub fn resource_state(&self, unit_id: &str, generation: u64) -> Option<RuntimeUnitState> {
        self.resources
            .lock()
            .expect("provider resource lock")
            .get(unit_id)
            .and_then(|generations| generations.get(&generation))
            .map(|resource| resource.observation.state)
    }

    pub fn effect_count(&self, operation: FaultOperation) -> usize {
        self.effects
            .lock()
            .expect("effect counter lock")
            .get(&operation)
            .copied()
            .unwrap_or_default()
    }

    async fn boundary(
        &self,
        operation: FaultOperation,
        boundary: FaultBoundary,
    ) -> RuntimeResult<()> {
        self.trace.lock().expect("trace lock").push(TraceEvent {
            operation,
            boundary,
        });
        let mode = {
            let mut armed = self.fault.lock().expect("fault plan lock");
            if armed
                .as_ref()
                .is_some_and(|fault| fault.operation == operation && fault.boundary == boundary)
            {
                armed.take().map(|fault| fault.mode)
            } else {
                None
            }
        };
        match mode {
            None => Ok(()),
            Some(FaultMode::Transport) => Err(RuntimeError::Transport(format!(
                "injected {operation:?} {boundary:?} transport fault"
            ))),
            Some(FaultMode::Hang) => std::future::pending::<RuntimeResult<()>>().await,
            Some(FaultMode::Panic) => {
                panic!("injected {operation:?} {boundary:?} provider panic")
            }
        }
    }

    fn record_effect(&self, operation: FaultOperation) {
        *self
            .effects
            .lock()
            .expect("effect counter lock")
            .entry(operation)
            .or_default() += 1;
    }

    fn capabilities_value(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            schema: RuntimeCapabilities::SCHEMA.into(),
            provider_id: self.provider_id.clone(),
            provider_build: "deterministic-fault-driver/1".into(),
            unit_classes: vec![RuntimeUnitClass::Task, RuntimeUnitClass::Service],
            artifact_media_types: vec![IMAGE_MEDIA_TYPE.into()],
            isolation_levels: vec![IsolationLevel::Container],
            network_modes: vec![NetworkMode::None],
            mount_kinds: Vec::new(),
            health_check_kinds: Vec::new(),
            resource_controls: vec![
                ResourceControl::Cpu,
                ResourceControl::Memory,
                ResourceControl::Pids,
                ResourceControl::ExecutionTimeout,
            ],
            features: vec![
                RuntimeFeature::DurableIdentity,
                RuntimeFeature::Stop,
                RuntimeFeature::Remove,
                RuntimeFeature::Logs,
                RuntimeFeature::Exec,
            ],
        }
    }

    fn provider_observation(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
        resource_id: String,
    ) -> RuntimeObservation {
        let mut observation = current.clone();
        observation.state = match spec.class {
            RuntimeUnitClass::Task => RuntimeUnitState::Succeeded,
            RuntimeUnitClass::Service => RuntimeUnitState::Running,
        };
        observation.provider_resource_id = Some(resource_id);
        observation.provider_build = Some("deterministic-fault-driver/1".into());
        observation.observed_at_ms = current.observed_at_ms.saturating_add(1);
        observation.started_at_ms = Some(current.observed_at_ms);
        observation.finished_at_ms = observation
            .state
            .is_terminal()
            .then_some(observation.observed_at_ms);
        observation.health = None;
        observation.outputs.clear();
        observation.failure = None;
        observation
    }

    fn exec_key(request: &RuntimeExecRequest) -> String {
        format!(
            "{}/{}/{}",
            request.unit_id, request.generation, request.request_id
        )
    }
}

#[async_trait]
impl RuntimeDriver for DeterministicFaultDriver {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        self.boundary(FaultOperation::Capabilities, FaultBoundary::BeforeEffect)
            .await?;
        let capabilities = self.capabilities_value();
        self.boundary(FaultOperation::Capabilities, FaultBoundary::AfterEffect)
            .await?;
        Ok(capabilities)
    }

    async fn apply(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeObservation> {
        self.boundary(FaultOperation::Apply, FaultBoundary::BeforeEffect)
            .await?;
        let (observation, created) = {
            let mut resources = self.resources.lock().expect("provider resource lock");
            let generations = resources.entry(spec.unit_id.clone()).or_default();
            let mut created = false;
            let observation = if let Some(resource) = generations.get(&spec.generation) {
                resource.observation.clone()
            } else {
                created = true;
                self.provider_observation(
                    spec,
                    current,
                    format!("provider/{}/g{}", spec.unit_id, spec.generation),
                )
            };
            generations.insert(
                spec.generation,
                ProviderResource {
                    observation: observation.clone(),
                },
            );
            let stale = generations
                .keys()
                .copied()
                .filter(|generation| *generation != spec.generation)
                .collect::<BTreeSet<_>>();
            for generation in stale {
                generations.remove(&generation);
            }
            (observation, created)
        };
        if created {
            self.record_effect(FaultOperation::Apply);
        }
        self.boundary(FaultOperation::Apply, FaultBoundary::AfterEffect)
            .await?;
        Ok(observation)
    }

    async fn inspect(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection> {
        self.boundary(FaultOperation::Inspect, FaultBoundary::BeforeEffect)
            .await?;
        let inspection = self
            .resources
            .lock()
            .expect("provider resource lock")
            .get(&unit.spec.unit_id)
            .and_then(|generations| generations.get(&unit.spec.generation))
            .map_or_else(
                || RuntimeInspection::NotFound {
                    schema: RuntimeInspection::SCHEMA.into(),
                    unit_id: unit.spec.unit_id.clone(),
                    last_generation: Some(unit.spec.generation),
                },
                |resource| RuntimeInspection::Found {
                    schema: RuntimeInspection::SCHEMA.into(),
                    observation: Box::new(resource.observation.clone()),
                },
            );
        self.boundary(FaultOperation::Inspect, FaultBoundary::AfterEffect)
            .await?;
        Ok(inspection)
    }

    async fn stop(
        &self,
        unit: &RuntimeUnitRecord,
        _request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeObservation> {
        self.boundary(FaultOperation::Stop, FaultBoundary::BeforeEffect)
            .await?;
        let (observation, changed) = {
            let mut resources = self.resources.lock().expect("provider resource lock");
            let resource = resources
                .get_mut(&unit.spec.unit_id)
                .and_then(|generations| generations.get_mut(&unit.spec.generation))
                .ok_or_else(|| RuntimeError::NotFound {
                    unit_id: unit.spec.unit_id.clone(),
                })?;
            let changed = resource.observation.state != RuntimeUnitState::Stopped;
            if changed {
                resource.observation.state = RuntimeUnitState::Stopped;
                resource.observation.observed_at_ms =
                    resource.observation.observed_at_ms.saturating_add(1);
                resource.observation.finished_at_ms = Some(resource.observation.observed_at_ms);
                resource.observation.health = None;
            }
            (resource.observation.clone(), changed)
        };
        if changed {
            self.record_effect(FaultOperation::Stop);
        }
        self.boundary(FaultOperation::Stop, FaultBoundary::AfterEffect)
            .await?;
        Ok(observation)
    }

    async fn remove(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeRemoval> {
        self.boundary(FaultOperation::Remove, FaultBoundary::BeforeEffect)
            .await?;
        let existed = {
            let mut resources = self.resources.lock().expect("provider resource lock");
            let removed = resources
                .get_mut(&unit.spec.unit_id)
                .and_then(|generations| generations.remove(&unit.spec.generation))
                .is_some();
            if resources
                .get(&unit.spec.unit_id)
                .is_some_and(BTreeMap::is_empty)
            {
                resources.remove(&unit.spec.unit_id);
            }
            removed
        };
        if existed {
            self.record_effect(FaultOperation::Remove);
        }
        let removal = RuntimeRemoval {
            schema: RuntimeRemoval::SCHEMA.into(),
            request_id: request.request_id.clone(),
            unit_id: unit.spec.unit_id.clone(),
            generation: unit.spec.generation,
            removed_at_ms: unit.observation.observed_at_ms.saturating_add(1),
            already_absent: !existed,
        };
        self.boundary(FaultOperation::Remove, FaultBoundary::AfterEffect)
            .await?;
        Ok(removal)
    }

    async fn logs(
        &self,
        unit: &RuntimeUnitRecord,
        _query: &RuntimeLogQuery,
    ) -> RuntimeResult<Vec<RuntimeLogChunk>> {
        self.boundary(FaultOperation::Logs, FaultBoundary::BeforeEffect)
            .await?;
        let chunks = vec![RuntimeLogChunk {
            schema: RuntimeLogChunk::SCHEMA.into(),
            cursor: "fault-cursor-1".into(),
            sequence: 1,
            observed_at_ms: unit.observation.observed_at_ms,
            stream: RuntimeLogStream::Stdout,
            data: "ready\n".into(),
        }];
        self.boundary(FaultOperation::Logs, FaultBoundary::AfterEffect)
            .await?;
        Ok(chunks)
    }

    async fn exec(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeExecRequest,
    ) -> RuntimeResult<RuntimeExecResult> {
        self.boundary(FaultOperation::Exec, FaultBoundary::BeforeEffect)
            .await?;
        let key = Self::exec_key(request);
        let (result, created) = {
            let mut results = self.exec_results.lock().expect("exec result lock");
            if let Some(result) = results.get(&key) {
                (result.clone(), false)
            } else {
                let result = RuntimeExecResult {
                    schema: RuntimeExecResult::SCHEMA.into(),
                    request_id: request.request_id.clone(),
                    observation: unit.observation.clone(),
                    exit_code: 0,
                    stdout: "ok\n".into(),
                    stderr: String::new(),
                    truncated: false,
                };
                results.insert(key, result.clone());
                (result, true)
            }
        };
        if created {
            self.record_effect(FaultOperation::Exec);
        }
        self.boundary(FaultOperation::Exec, FaultBoundary::AfterEffect)
            .await?;
        Ok(result)
    }
}
