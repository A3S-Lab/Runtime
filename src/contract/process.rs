use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProcessSpec {
    /// An empty command uses the runnable artifact's declared entrypoint.
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub environment: BTreeMap<String, String>,
}

impl RuntimeProcessSpec {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.command.len() > 64 || self.args.len() > 256 {
            return Err("process command or argument count exceeds Runtime limits".into());
        }
        for value in self.command.iter().chain(self.args.iter()) {
            validate_process_value(value)?;
        }
        if let Some(path) = &self.working_directory {
            super::validate_absolute_path("working_directory", path)?;
        }
        if self.environment.len() > 512 {
            return Err("process environment exceeds 512 entries".into());
        }
        for (name, value) in &self.environment {
            validate_environment_name(name)?;
            if value.len() > 32 * 1024 || value.contains('\0') {
                return Err(format!("environment value for {name:?} is invalid"));
            }
        }
        Ok(())
    }
}

fn validate_process_value(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 32 * 1024 || value.contains('\0') {
        return Err("process command values must be nonempty bounded non-NUL strings".into());
    }
    Ok(())
}

pub(crate) fn validate_environment_name(value: &str) -> Result<(), String> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err("environment variable name must not be empty".into());
    };
    if !(first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        || value.len() > 255
    {
        return Err(format!("invalid environment variable name {value:?}"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretTarget {
    Environment { variable: String },
    File { path: String, mode: u32 },
    RegistryCredential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretReference {
    pub name: String,
    /// Opaque reference resolved by the provider integration. It is never the
    /// secret value itself.
    pub reference: String,
    pub target: SecretTarget,
}

impl SecretReference {
    pub(crate) fn validate(&self) -> Result<(), String> {
        super::validate_name("secret name", &self.name)?;
        super::validate_nonempty("secret reference", &self.reference, 1024)?;
        match &self.target {
            SecretTarget::Environment { variable } => validate_environment_name(variable),
            SecretTarget::File { path, mode } => {
                super::validate_absolute_path("secret file path", path)?;
                if *mode == 0 || *mode > 0o777 {
                    return Err("secret file mode must be between 0001 and 0777".into());
                }
                Ok(())
            }
            SecretTarget::RegistryCredential => Ok(()),
        }
    }
}
