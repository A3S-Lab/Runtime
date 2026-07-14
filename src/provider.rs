use crate::{RuntimeError, RuntimeResult};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn parse(value: impl Into<String>) -> RuntimeResult<Self> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || (byte == b'-' && index > 0 && index + 1 < value.len())
            });
        if !valid {
            return Err(RuntimeError::InvalidRequest(format!(
                "Runtime provider ID {value:?} must use lowercase ASCII letters, digits, and interior hyphens"
            )));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for ProviderId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_are_portable_and_have_no_default_policy() {
        for invalid in ["", "Docker", "a3s_box", "-docker", "docker-", "a/b"] {
            assert!(ProviderId::parse(invalid).is_err(), "accepted {invalid:?}");
        }
        assert_eq!(
            ProviderId::parse("vendor-runtime-2").unwrap().as_str(),
            "vendor-runtime-2"
        );
    }
}
