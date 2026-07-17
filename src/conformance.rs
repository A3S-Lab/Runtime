use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeInspection, RuntimeObservation,
    RuntimeRemoval, RuntimeUnitClass, RuntimeUnitState,
};
use crate::{RuntimeClient, RuntimeError, RuntimeResult};

/// Provider-owned inputs for the destructive Runtime conformance suite.
///
/// Unit and request IDs must be unique to the suite invocation. Providers may
/// create real resources, so callers should use disposable artifacts and an
/// isolated provider namespace.
#[derive(Debug, Clone)]
pub struct RuntimeConformanceCase {
    pub task_apply: RuntimeApplyRequest,
    pub task_remove: RuntimeActionRequest,
    pub service_apply: RuntimeApplyRequest,
    pub service_stop: RuntimeActionRequest,
    pub service_remove: RuntimeActionRequest,
}

impl RuntimeConformanceCase {
    pub fn validate(&self) -> Result<(), String> {
        self.task_apply.validate()?;
        self.task_remove.validate()?;
        self.service_apply.validate()?;
        self.service_stop.validate()?;
        self.service_remove.validate()?;
        if self.task_apply.spec.class != RuntimeUnitClass::Task {
            return Err("conformance task_apply must describe a Task".into());
        }
        if self.service_apply.spec.class != RuntimeUnitClass::Service {
            return Err("conformance service_apply must describe a Service".into());
        }
        if self.task_apply.spec.unit_id == self.service_apply.spec.unit_id {
            return Err("conformance Task and Service must use different unit IDs".into());
        }
        validate_action(&self.task_remove, &self.task_apply)?;
        validate_action(&self.service_stop, &self.service_apply)?;
        validate_action(&self.service_remove, &self.service_apply)?;
        let mut request_ids = [
            self.task_apply.request_id.as_str(),
            self.task_remove.request_id.as_str(),
            self.service_apply.request_id.as_str(),
            self.service_stop.request_id.as_str(),
            self.service_remove.request_id.as_str(),
        ];
        request_ids.sort_unstable();
        if request_ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("conformance requests must use unique request IDs".into());
        }
        Ok(())
    }
}

/// Evidence returned after a provider passes the common Task and Service path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConformanceReport {
    pub task: RuntimeObservation,
    pub task_removal: RuntimeRemoval,
    pub service: RuntimeObservation,
    pub stopped_service: RuntimeObservation,
    pub service_removal: RuntimeRemoval,
}

/// Runs the destructive provider-neutral lifecycle conformance suite.
///
/// Provider-specific tests remain responsible for crash injection and resource
/// reconstruction. This suite establishes the shared protocol semantics those
/// fault tests must preserve.
pub async fn verify_runtime_provider(
    client: &dyn RuntimeClient,
    case: &RuntimeConformanceCase,
) -> RuntimeResult<RuntimeConformanceReport> {
    case.validate().map_err(RuntimeError::InvalidRequest)?;
    let capabilities = client.capabilities().await?;
    capabilities.validate().map_err(RuntimeError::Protocol)?;
    for spec in [&case.task_apply.spec, &case.service_apply.spec] {
        let missing = capabilities
            .missing_for(spec)
            .map_err(RuntimeError::Protocol)?;
        if !missing.is_empty() {
            return Err(RuntimeError::UnsupportedCapabilities(missing));
        }
    }

    let task = client.apply(&case.task_apply).await?;
    task.validate_against(&case.task_apply.spec)
        .map_err(RuntimeError::Protocol)?;
    if !task.converges(&case.task_apply.spec) {
        return Err(RuntimeError::Protocol(
            "conformance Task did not reach succeeded".into(),
        ));
    }
    require_equal(
        "duplicate Task apply",
        &task,
        &client.apply(&case.task_apply).await?,
    )?;
    let inspected_task = require_found("inspect Task", client.inspect(&task.unit_id).await?)?;
    require_equal("terminal Task inspection", &task, &inspected_task)?;
    let task_removal = client.remove(&case.task_remove).await?;
    require_equal(
        "duplicate Task remove",
        &task_removal,
        &client.remove(&case.task_remove).await?,
    )?;
    require_absent(
        "removed Task inspection",
        client.inspect(&task.unit_id).await?,
        task.generation,
    )?;

    let service = client.apply(&case.service_apply).await?;
    service
        .validate_against(&case.service_apply.spec)
        .map_err(RuntimeError::Protocol)?;
    if !service.converges(&case.service_apply.spec) {
        return Err(RuntimeError::Protocol(
            "conformance Service did not reach running and healthy".into(),
        ));
    }
    require_equal(
        "duplicate Service apply",
        &service,
        &client.apply(&case.service_apply).await?,
    )?;
    let inspected_service =
        require_found("inspect Service", client.inspect(&service.unit_id).await?)?;
    inspected_service
        .validate_against(&case.service_apply.spec)
        .map_err(RuntimeError::Protocol)?;
    let stopped_service = require_found("stop Service", client.stop(&case.service_stop).await?)?;
    if stopped_service.state != RuntimeUnitState::Stopped {
        return Err(RuntimeError::Protocol(
            "conformance Service stop did not reach stopped".into(),
        ));
    }
    let duplicate_stop = require_found(
        "duplicate Service stop",
        client.stop(&case.service_stop).await?,
    )?;
    require_equal("duplicate Service stop", &stopped_service, &duplicate_stop)?;
    let service_removal = client.remove(&case.service_remove).await?;
    require_equal(
        "duplicate Service remove",
        &service_removal,
        &client.remove(&case.service_remove).await?,
    )?;
    require_absent(
        "removed Service inspection",
        client.inspect(&service.unit_id).await?,
        service.generation,
    )?;

    Ok(RuntimeConformanceReport {
        task,
        task_removal,
        service,
        stopped_service,
        service_removal,
    })
}

fn validate_action(
    action: &RuntimeActionRequest,
    apply: &RuntimeApplyRequest,
) -> Result<(), String> {
    if action.unit_id != apply.spec.unit_id || action.generation != apply.spec.generation {
        return Err(format!(
            "conformance action {:?} does not target apply request {:?}",
            action.request_id, apply.request_id
        ));
    }
    Ok(())
}

fn require_found(label: &str, inspection: RuntimeInspection) -> RuntimeResult<RuntimeObservation> {
    match inspection {
        RuntimeInspection::Found { observation, .. } => Ok(*observation),
        RuntimeInspection::NotFound { .. } => Err(RuntimeError::Protocol(format!(
            "{label} unexpectedly returned not found"
        ))),
    }
}

fn require_absent(
    label: &str,
    inspection: RuntimeInspection,
    generation: u64,
) -> RuntimeResult<()> {
    match inspection {
        RuntimeInspection::NotFound {
            last_generation: Some(last_generation),
            ..
        } if last_generation == generation => Ok(()),
        _ => Err(RuntimeError::Protocol(format!(
            "{label} did not preserve the removed generation"
        ))),
    }
}

fn require_equal<T>(label: &str, expected: &T, actual: &T) -> RuntimeResult<()>
where
    T: PartialEq,
{
    if expected != actual {
        return Err(RuntimeError::Protocol(format!(
            "{label} returned a different durable result"
        )));
    }
    Ok(())
}
