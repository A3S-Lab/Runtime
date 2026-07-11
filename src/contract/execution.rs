use super::{ArtifactRef, OutputArtifact, PrivacyClass, ProtectedMount};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRole {
    Candidate,
    Judge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    pub wall_time_ms: u64,
    pub cpu_millis: u64,
    pub memory_bytes: u64,
    pub scratch_bytes: u64,
    pub output_bytes: u64,
}

impl ResourceLimits {
    fn validate(&self) -> Result<(), String> {
        if self.wall_time_ms == 0
            || self.cpu_millis == 0
            || self.memory_bytes == 0
            || self.scratch_bytes == 0
            || self.output_bytes == 0
        {
            return Err("all Runtime resource limits must be positive".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionPolicy {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub max_files: u64,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
}

impl SubmissionPolicy {
    fn validate(&self) -> Result<(), String> {
        if self.max_files == 0
            || self.max_file_bytes == 0
            || self.max_total_bytes < self.max_file_bytes
        {
            return Err("submission limits are invalid".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExecutionSpec {
    pub schema: String,
    pub operation_id: String,
    pub role: RuntimeRole,
    pub asset: ArtifactRef,
    pub work_image: ArtifactRef,
    pub protected_mounts: Vec<ProtectedMount>,
    pub protected_result_schema: Option<String>,
    pub submission_policy: Option<SubmissionPolicy>,
    pub network: NetworkPolicy,
    pub resources: ResourceLimits,
}

impl RuntimeExecutionSpec {
    pub const SCHEMA: &'static str = "a3s.runtime.execution-spec.v1";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime execution schema {:?}",
                self.schema
            ));
        }
        if self.operation_id.trim().is_empty() {
            return Err("operation_id must not be empty".into());
        }
        self.asset.validate()?;
        self.work_image.validate()?;
        self.resources.validate()?;
        for mount in &self.protected_mounts {
            if mount.name.trim().is_empty() || !mount.read_only {
                return Err("protected mounts must be named and read-only".into());
            }
            mount.artifact.validate()?;
        }
        match self.role {
            RuntimeRole::Candidate => {
                if self.protected_result_schema.is_some() || self.submission_policy.is_none() {
                    return Err(
                        "Candidate requires submission_policy and no protected result".into(),
                    );
                }
                self.submission_policy.as_ref().unwrap().validate()?;
            }
            RuntimeRole::Judge => {
                if self
                    .protected_result_schema
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
                    || self.submission_policy.is_some()
                {
                    return Err("Judge requires a protected result and no submission_policy".into());
                }
                let submissions = self
                    .protected_mounts
                    .iter()
                    .filter(|mount| mount.privacy == PrivacyClass::TrialSubmission)
                    .count();
                if submissions != 1 {
                    return Err("Judge requires exactly one SubmissionSnapshot mount".into());
                }
            }
        }
        Ok(())
    }

    /// Digest of the closed semantic request. The schema contains no maps or
    /// transport-only fields, so Serde's declared field order is canonical for
    /// this protocol version.
    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| format!("could not encode Runtime execution spec: {error}"))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl ExecutionState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeUsage {
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub peak_memory_bytes: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEvidence {
    pub semantics_profile_digest: String,
    pub provider_build: String,
    pub spec_digest: String,
    pub claims: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExecutionResult {
    pub schema: String,
    pub execution_id: String,
    pub operation_id: String,
    pub spec_digest: String,
    pub role: RuntimeRole,
    pub state: ExecutionState,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub typed_result_artifact: Option<OutputArtifact>,
    pub terminal_checkpoint: Option<OutputArtifact>,
    pub submission_snapshot: Option<OutputArtifact>,
    pub usage: Option<RuntimeUsage>,
    pub evidence: Option<RuntimeEvidence>,
    pub provider_attestation: Option<ArtifactRef>,
    pub failure: Option<ExecutionFailure>,
}

impl RuntimeExecutionResult {
    pub const SCHEMA: &'static str = "a3s.runtime.execution-result.v1";

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != Self::SCHEMA {
            return Err(format!(
                "unsupported Runtime result schema {:?}",
                self.schema
            ));
        }
        if self.execution_id.trim().is_empty() || self.operation_id.trim().is_empty() {
            return Err("execution and operation IDs must not be empty".into());
        }
        super::artifact::validate_digest(&self.spec_digest)?;
        if let (Some(started), Some(finished)) = (self.started_at_ms, self.finished_at_ms) {
            if finished < started {
                return Err("finished_at_ms precedes started_at_ms".into());
            }
        }
        if !self.state.is_terminal() {
            if self.finished_at_ms.is_some()
                || self.typed_result_artifact.is_some()
                || self.terminal_checkpoint.is_some()
                || self.submission_snapshot.is_some()
                || self.failure.is_some()
            {
                return Err("nonterminal result contains terminal fields".into());
            }
            return Ok(());
        }
        if self.finished_at_ms.is_none() || self.usage.is_none() || self.evidence.is_none() {
            return Err("terminal result is missing usage or evidence".into());
        }
        let evidence = self.evidence.as_ref().unwrap();
        super::artifact::validate_digest(&evidence.semantics_profile_digest)?;
        super::artifact::validate_digest(&evidence.spec_digest)?;
        if evidence.spec_digest != self.spec_digest || evidence.provider_build.trim().is_empty() {
            return Err("Runtime evidence does not bind the result spec and provider".into());
        }
        if let Some(attestation) = &self.provider_attestation {
            attestation.validate()?;
        }
        match self.state {
            ExecutionState::Succeeded => {
                if self.failure.is_some() {
                    return Err("successful result contains failure".into());
                }
                match self.role {
                    RuntimeRole::Candidate
                        if self.terminal_checkpoint.is_some()
                            && self.submission_snapshot.is_some()
                            && self.typed_result_artifact.is_none() =>
                    {
                        validate_output(
                            self.terminal_checkpoint.as_ref().unwrap(),
                            PrivacyClass::CandidatePrivate,
                        )?;
                        validate_output(
                            self.submission_snapshot.as_ref().unwrap(),
                            PrivacyClass::TrialSubmission,
                        )?;
                    }
                    RuntimeRole::Judge
                        if self.typed_result_artifact.is_some()
                            && self.terminal_checkpoint.is_none()
                            && self.submission_snapshot.is_none() =>
                    {
                        validate_output(
                            self.typed_result_artifact.as_ref().unwrap(),
                            PrivacyClass::ProtectedResult,
                        )?;
                    }
                    _ => return Err("successful result violates role artifact contract".into()),
                }
            }
            ExecutionState::Failed | ExecutionState::Cancelled => {
                if self.failure.is_none()
                    || self.typed_result_artifact.is_some()
                    || self.terminal_checkpoint.is_some()
                    || self.submission_snapshot.is_some()
                {
                    return Err("failed result violates terminal artifact contract".into());
                }
            }
            ExecutionState::Queued | ExecutionState::Running => unreachable!(),
        }
        Ok(())
    }
}

fn validate_output(output: &OutputArtifact, privacy: PrivacyClass) -> Result<(), String> {
    output.artifact.validate()?;
    if output.privacy != privacy {
        return Err("Runtime output has the wrong privacy class".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact() -> ArtifactRef {
        ArtifactRef {
            digest: format!("sha256:{}", "a".repeat(64)),
            media_type: "application/vnd.a3s.asset.v1".into(),
        }
    }

    fn resources() -> ResourceLimits {
        ResourceLimits {
            wall_time_ms: 1,
            cpu_millis: 1,
            memory_bytes: 1,
            scratch_bytes: 1,
            output_bytes: 1,
        }
    }

    #[test]
    fn role_specific_specs_fail_closed() {
        let candidate = RuntimeExecutionSpec {
            schema: RuntimeExecutionSpec::SCHEMA.into(),
            operation_id: "run/candidate".into(),
            role: RuntimeRole::Candidate,
            asset: artifact(),
            work_image: artifact(),
            protected_mounts: vec![],
            protected_result_schema: None,
            submission_policy: Some(SubmissionPolicy {
                include: vec!["**".into()],
                exclude: vec![],
                max_files: 1,
                max_total_bytes: 1,
                max_file_bytes: 1,
            }),
            network: NetworkPolicy::None,
            resources: resources(),
        };
        candidate.validate().unwrap();
        assert_eq!(candidate.digest().unwrap(), candidate.digest().unwrap());
        let mut changed = candidate.clone();
        changed.operation_id = "run/other-candidate".into();
        assert_ne!(candidate.digest().unwrap(), changed.digest().unwrap());
        let mut judge = candidate.clone();
        judge.role = RuntimeRole::Judge;
        assert!(judge.validate().is_err());
        judge.submission_policy = None;
        judge.protected_result_schema = Some("bench.judge.result.v1".into());
        judge.protected_mounts.push(ProtectedMount {
            name: "submission".into(),
            artifact: artifact(),
            privacy: PrivacyClass::TrialSubmission,
            read_only: true,
        });
        judge.validate().unwrap();
    }

    #[test]
    fn closed_result_schema_and_role_artifacts_are_enforced() {
        let unknown = format!(
            r#"{{"schema":"{}","unknown":true}}"#,
            RuntimeExecutionResult::SCHEMA
        );
        assert!(serde_json::from_str::<RuntimeExecutionResult>(&unknown).is_err());

        let result = RuntimeExecutionResult {
            schema: RuntimeExecutionResult::SCHEMA.into(),
            execution_id: "execution-1".into(),
            operation_id: "run/candidate".into(),
            spec_digest: artifact().digest,
            role: RuntimeRole::Candidate,
            state: ExecutionState::Succeeded,
            started_at_ms: Some(1),
            finished_at_ms: Some(2),
            typed_result_artifact: None,
            terminal_checkpoint: Some(OutputArtifact {
                artifact: artifact(),
                privacy: PrivacyClass::CandidatePrivate,
            }),
            submission_snapshot: Some(OutputArtifact {
                artifact: artifact(),
                privacy: PrivacyClass::TrialSubmission,
            }),
            usage: Some(RuntimeUsage {
                wall_time_ms: 1,
                cpu_time_ms: 1,
                peak_memory_bytes: 1,
                input_tokens: 0,
                output_tokens: 0,
            }),
            evidence: Some(RuntimeEvidence {
                semantics_profile_digest: artifact().digest,
                provider_build: "test".into(),
                spec_digest: artifact().digest,
                claims: BTreeMap::new(),
            }),
            provider_attestation: None,
            failure: None,
        };
        result.validate().unwrap();
        let mut invalid = result;
        invalid.typed_result_artifact = Some(OutputArtifact {
            artifact: artifact(),
            privacy: PrivacyClass::ProtectedResult,
        });
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn result_evidence_and_privacy_are_identity_bound() {
        let mut result = RuntimeExecutionResult {
            schema: RuntimeExecutionResult::SCHEMA.into(),
            execution_id: "execution-1".into(),
            operation_id: "run/judge".into(),
            spec_digest: artifact().digest,
            role: RuntimeRole::Judge,
            state: ExecutionState::Succeeded,
            started_at_ms: Some(2),
            finished_at_ms: Some(3),
            typed_result_artifact: Some(OutputArtifact {
                artifact: artifact(),
                privacy: PrivacyClass::ProtectedResult,
            }),
            terminal_checkpoint: None,
            submission_snapshot: None,
            usage: Some(RuntimeUsage {
                wall_time_ms: 1,
                cpu_time_ms: 1,
                peak_memory_bytes: 1,
                input_tokens: 0,
                output_tokens: 0,
            }),
            evidence: Some(RuntimeEvidence {
                semantics_profile_digest: artifact().digest,
                provider_build: "test".into(),
                spec_digest: artifact().digest,
                claims: BTreeMap::new(),
            }),
            provider_attestation: None,
            failure: None,
        };
        result.validate().unwrap();
        result.typed_result_artifact.as_mut().unwrap().privacy = PrivacyClass::Public;
        assert!(result.validate().is_err());
        result.typed_result_artifact.as_mut().unwrap().privacy = PrivacyClass::ProtectedResult;
        result.evidence.as_mut().unwrap().spec_digest = format!("sha256:{}", "b".repeat(64));
        assert!(result.validate().is_err());
    }
}
