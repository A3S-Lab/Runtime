use crate::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeCapabilities, RuntimeExecRequest,
    RuntimeExecResult, RuntimeInspection, RuntimeLogChunk, RuntimeLogQuery, RuntimeObservation,
    RuntimeRemoval, RuntimeSnapshot, RuntimeSnapshotRequest, RuntimeUnitSpec,
};
use crate::{ProviderId, RuntimeError, RuntimeResult, RuntimeUnitRecord};
use async_trait::async_trait;

/// Provider-specific primitive used by `ManagedRuntimeClient`.
///
/// Drivers do not own request idempotency or durable shared state. `apply`,
/// `stop`, and `remove` must nevertheless be safe to reattach after an
/// ambiguous transport failure using the stable unit identity and generation.
/// A successful `apply` must also retire every older provider generation for
/// the unit before returning, leaving exactly one managed provider resource.
/// If that handoff is interrupted, an exact retry must discover the partially
/// created current generation and finish the same reconciliation.
#[async_trait]
pub trait RuntimeDriver: Send + Sync {
    fn provider_id(&self) -> &ProviderId;

    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities>;

    async fn apply(
        &self,
        spec: &RuntimeUnitSpec,
        current: &RuntimeObservation,
    ) -> RuntimeResult<RuntimeObservation>;

    async fn inspect(&self, unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection>;

    async fn stop(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeObservation>;

    async fn remove(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeActionRequest,
    ) -> RuntimeResult<RuntimeRemoval>;

    /// ED3: capture the unit's state to a keyed snapshot before
    /// termination. The `operation_id` is the caller's at-most-once
    /// key — a replay of the same id returns the same receipt without
    /// recapturing. Drivers that cannot snapshot keep this default and
    /// refuse with `UnsupportedCapabilities`; the control plane only
    /// asks engines that advertise
    /// [`RuntimeFeature::StateSnapshot`](crate::contract::RuntimeFeature::StateSnapshot).
    async fn snapshot(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeSnapshotRequest,
    ) -> RuntimeResult<RuntimeSnapshot> {
        let _ = (unit, request);
        Err(RuntimeError::UnsupportedCapabilities(vec![
            "StateSnapshot".into()
        ]))
    }

    async fn logs(
        &self,
        unit: &RuntimeUnitRecord,
        query: &RuntimeLogQuery,
    ) -> RuntimeResult<Vec<RuntimeLogChunk>>;

    /// Executes one durably identified request within its original budget.
    ///
    /// `ManagedRuntimeClient` always supplies `request.deadline_at_ms` as the
    /// effective absolute deadline captured by the first reservation (see
    /// [`RuntimeExecRequest::effective_deadline_at_ms`](crate::contract::RuntimeExecRequest::effective_deadline_at_ms)).
    /// Relative-only Exec budgets include a control-plane slack so short
    /// `timeout_ms` values remain guest-command budgets rather than being eaten
    /// by lease/capability waits. A pending replay receives the same persisted
    /// value, so a driver must not restart or extend the execution window.
    /// Drivers may enforce a shorter provider-specific timeout (typically the
    /// original `timeout_ms`) and must deduplicate or reattach the stable
    /// request ID after an ambiguous result.
    async fn exec(
        &self,
        unit: &RuntimeUnitRecord,
        request: &RuntimeExecRequest,
    ) -> RuntimeResult<RuntimeExecResult>;
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    struct NoSnapshotDriver;

    #[async_trait]
    impl RuntimeDriver for NoSnapshotDriver {
        fn provider_id(&self) -> &ProviderId {
            static PROVIDER: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
            PROVIDER.get_or_init(|| ProviderId::parse("no-snapshot").expect("provider"))
        }

        async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
            Err(RuntimeError::UnsupportedCapabilities(vec![
                "capabilities".into()
            ]))
        }

        async fn apply(
            &self,
            _spec: &RuntimeUnitSpec,
            _current: &RuntimeObservation,
        ) -> RuntimeResult<RuntimeObservation> {
            unimplemented!("apply is not under test")
        }

        async fn inspect(&self, _unit: &RuntimeUnitRecord) -> RuntimeResult<RuntimeInspection> {
            unimplemented!("inspect is not under test")
        }

        async fn stop(
            &self,
            _unit: &RuntimeUnitRecord,
            _request: &RuntimeActionRequest,
        ) -> RuntimeResult<RuntimeObservation> {
            unimplemented!("stop is not under test")
        }

        async fn remove(
            &self,
            _unit: &RuntimeUnitRecord,
            _request: &RuntimeActionRequest,
        ) -> RuntimeResult<RuntimeRemoval> {
            unimplemented!("remove is not under test")
        }

        async fn logs(
            &self,
            _unit: &RuntimeUnitRecord,
            _query: &RuntimeLogQuery,
        ) -> RuntimeResult<Vec<RuntimeLogChunk>> {
            unimplemented!("logs is not under test")
        }

        async fn exec(
            &self,
            _unit: &RuntimeUnitRecord,
            _request: &RuntimeExecRequest,
        ) -> RuntimeResult<RuntimeExecResult> {
            unimplemented!("exec is not under test")
        }
    }

    fn request() -> RuntimeSnapshotRequest {
        RuntimeSnapshotRequest {
            schema: RuntimeSnapshotRequest::SCHEMA.into(),
            operation_id: "handoff:drain-1".into(),
            unit_id: "unit-1".into(),
            generation: 3,
            include_memory: true,
            deadline_at_ms: Some(60_000),
        }
    }

    #[test]
    fn snapshot_request_validates_and_round_trips() {
        let valid = request();
        assert!(valid.validate().is_ok());
        let encoded = serde_json::to_vec(&valid).expect("encode");
        let decoded: RuntimeSnapshotRequest = serde_json::from_slice(&encoded).expect("decode");
        assert_eq!(decoded, valid);
        let mut wrong_schema = valid.clone();
        wrong_schema.schema = "a3s.runtime.snapshot.v0".into();
        assert!(wrong_schema.validate().is_err());
        let mut zero_generation = valid;
        zero_generation.generation = 0;
        assert!(zero_generation.validate().is_err());
        let mut zero_deadline = request();
        zero_deadline.deadline_at_ms = Some(0);
        assert!(zero_deadline.validate().is_err());
    }

    #[test]
    fn snapshot_receipt_validates_and_round_trips() {
        let receipt = RuntimeSnapshot {
            schema: RuntimeSnapshot::SCHEMA.into(),
            operation_id: "handoff:drain-1".into(),
            unit_id: "unit-1".into(),
            generation: 3,
            include_memory: true,
            captured_at_ms: 1,
        };
        assert!(receipt.validate().is_ok());
        let encoded = serde_json::to_vec(&receipt).expect("encode");
        let decoded: RuntimeSnapshot = serde_json::from_slice(&encoded).expect("decode");
        assert_eq!(decoded, receipt);
        let mut wrong_schema = receipt;
        wrong_schema.schema = "a3s.runtime.snapshot.v0".into();
        assert!(wrong_schema.validate().is_err());
    }

    #[tokio::test]
    async fn drivers_without_the_feature_keep_the_default_refusal() {
        let driver = NoSnapshotDriver;
        let apply: RuntimeApplyRequest = serde_json::from_value(serde_json::json!({
            "schema": "a3s.runtime.apply-request.v1",
            "request_id": "snapshot-default-test",
            "spec": {
                "schema": "a3s.runtime.unit-spec.v4",
                "unit_id": "unit-1",
                "generation": 3,
                "class": "task",
                "artifact": {
                    "uri": "oci://registry.example/run@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "media_type": "application/vnd.oci.image.manifest.v1+json"
                },
                "process": {
                    "command": ["/unit"],
                    "args": [],
                    "environment": {}
                },
                "mounts": [],
                "secrets": [],
                "network": { "mode": "none", "ports": [] },
                "resources": {
                    "execution_timeout_ms": 60000,
                    "cpu_millis": 100,
                    "memory_bytes": 33554432,
                    "pids": 32
                },
                "isolation": "process",
                "health": null,
                "service_lifecycle": null,
                "restart": {"kind": "never"},
                "outputs": []
            }
        }))
        .expect("apply request");
        let unit = RuntimeUnitRecord::new(&apply, 1).expect("unit record");
        let error = driver
            .snapshot(&unit, &request())
            .await
            .expect_err("the default snapshot refuses");
        assert!(matches!(
            error,
            RuntimeError::UnsupportedCapabilities(features)
                if features == vec!["StateSnapshot".to_string()]
        ));
    }
}
