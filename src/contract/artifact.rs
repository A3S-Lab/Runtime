use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub digest: String,
    pub media_type: String,
}

impl ArtifactRef {
    pub fn validate(&self) -> Result<(), String> {
        validate_digest(&self.digest)?;
        if self.media_type.trim().is_empty() {
            return Err("artifact media_type must not be empty".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    Public,
    CandidatePrivate,
    JudgePrivate,
    TrialSubmission,
    ProtectedResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedMount {
    pub name: String,
    pub artifact: ArtifactRef,
    pub privacy: PrivacyClass,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputArtifact {
    pub artifact: ArtifactRef,
    pub privacy: PrivacyClass,
}

pub(crate) fn validate_digest(value: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err("digest must use sha256".into());
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("digest must contain exactly 64 hexadecimal characters".into());
    }
    Ok(())
}
