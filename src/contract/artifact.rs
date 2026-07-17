use serde::{Deserialize, Serialize};

/// Immutable artifact address understood by a Runtime provider.
///
/// `uri` tells the provider where to resolve the content while `digest` is the
/// authoritative identity. Providers must never replace the digest with a
/// mutable tag resolved at execution time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub uri: String,
    pub digest: String,
    pub media_type: String,
}

impl ArtifactRef {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_uri("artifact uri", &self.uri)?;
        super::validate_digest(&self.digest)?;
        super::validate_nonempty("artifact media_type", &self.media_type, 255)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOutputArtifact {
    pub name: String,
    pub artifact: ArtifactRef,
    pub size_bytes: u64,
}

impl RuntimeOutputArtifact {
    pub(crate) fn validate(&self) -> Result<(), String> {
        super::validate_name("output artifact name", &self.name)?;
        self.artifact.validate()
    }
}
