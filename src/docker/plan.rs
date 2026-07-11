use crate::{RuntimeError, RuntimeResult};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerMount {
    pub source: PathBuf,
    pub target: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerExecutionPlan {
    /// Immutable Docker image ID (`sha256:...`) or repo digest
    /// (`registry/repository@sha256:...`). Tags are rejected.
    pub image: String,
    pub platform: Option<String>,
    pub argv: Vec<String>,
    pub mounts: Vec<DockerMount>,
    pub environment: BTreeMap<String, String>,
}

impl DockerExecutionPlan {
    pub fn validate(&self) -> RuntimeResult<()> {
        validate_image(&self.image)?;
        if self.argv.is_empty() || self.argv.iter().any(|value| value.contains('\0')) {
            return Err(RuntimeError::InvalidRequest(
                "Docker argv must be non-empty and contain no NUL bytes".into(),
            ));
        }
        if let Some(platform) = &self.platform {
            let parts: Vec<_> = platform.split('/').collect();
            if parts.len() < 2
                || parts.len() > 3
                || parts.iter().any(|part| {
                    part.is_empty()
                        || !part.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                        })
                })
            {
                return Err(RuntimeError::InvalidRequest(
                    "Docker platform must use os/architecture[/variant] syntax".into(),
                ));
            }
        }
        let mut targets = std::collections::BTreeSet::new();
        for mount in &self.mounts {
            let metadata = std::fs::symlink_metadata(&mount.source).map_err(|error| {
                RuntimeError::InvalidRequest(format!(
                    "Docker mount source {} is unavailable: {error}",
                    mount.source.display()
                ))
            })?;
            if metadata.file_type().is_symlink()
                || (!metadata.is_dir() && !metadata.is_file())
                || mount.source.to_string_lossy().contains(',')
            {
                return Err(RuntimeError::InvalidRequest(format!(
                    "Docker mount source {} is unsafe",
                    mount.source.display()
                )));
            }
            validate_target(&mount.target)?;
            if !targets.insert(&mount.target) {
                return Err(RuntimeError::InvalidRequest(format!(
                    "duplicate Docker mount target {:?}",
                    mount.target
                )));
            }
        }
        for (key, value) in &self.environment {
            if !valid_environment_key(key) || value.contains('\0') || is_proxy_environment(key) {
                return Err(RuntimeError::InvalidRequest(format!(
                    "Docker environment entry {key:?} is unsafe"
                )));
            }
        }
        Ok(())
    }
}

fn validate_image(image: &str) -> RuntimeResult<()> {
    let digest = image
        .strip_prefix("sha256:")
        .or_else(|| image.rsplit_once("@sha256:").map(|(_, digest)| digest));
    if !matches!(digest, Some(value) if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(RuntimeError::InvalidRequest(
            "Docker image must be pinned by a complete sha256 digest".into(),
        ));
    }
    Ok(())
}

fn validate_target(target: &str) -> RuntimeResult<()> {
    if !target.starts_with('/')
        || target.contains(',')
        || target.contains('\0')
        || target
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(RuntimeError::InvalidRequest(format!(
            "Docker mount target {target:?} is not a normalized absolute path"
        )));
    }
    Ok(())
}

fn valid_environment_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_uppercase() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_proxy_environment(key: &str) -> bool {
    matches!(key, "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" | "NO_PROXY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mutable_images_unsafe_mounts_and_proxy_inheritance() {
        let directory = tempfile::tempdir().unwrap();
        let mut plan = DockerExecutionPlan {
            image: "alpine:latest".into(),
            platform: Some("linux/amd64".into()),
            argv: vec!["/bin/true".into()],
            mounts: vec![],
            environment: BTreeMap::new(),
        };
        assert!(plan.validate().is_err());
        plan.image = format!("sha256:{}", "a".repeat(64));
        plan.mounts.push(DockerMount {
            source: directory.path().to_path_buf(),
            target: "/workspace".into(),
            read_only: false,
        });
        plan.validate().unwrap();
        plan.mounts[0].target = "/workspace/../secret".into();
        assert!(plan.validate().is_err());
        plan.mounts[0].target = "/workspace".into();
        plan.environment
            .insert("HTTP_PROXY".into(), "http://host".into());
        assert!(plan.validate().is_err());
    }
}
