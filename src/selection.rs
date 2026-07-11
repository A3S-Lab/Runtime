use crate::{RuntimeError, RuntimeResult};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    pub const DOCKER: &'static str = "docker";
    pub const A3S_BOX: &'static str = "a3s-box";

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

    pub fn docker() -> Self {
        Self(Self::DOCKER.into())
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorRuntimeConfig {
    pub provider: Option<ProviderId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionRuntimePolicy {
    /// Provider selected by authenticated A3S OS policy. `None` also represents
    /// a signed-out caller: both cases fall back to local Docker when the
    /// operator did not make an explicit choice.
    pub provider: Option<ProviderId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionSource {
    OperatorConfig,
    SessionPolicy,
    SignedOutDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSelection {
    pub provider: ProviderId,
    pub source: SelectionSource,
}

impl RuntimeSelection {
    pub fn resolve(
        operator: &OperatorRuntimeConfig,
        session: &SessionRuntimePolicy,
    ) -> RuntimeSelection {
        if let Some(provider) = &operator.provider {
            return Self {
                provider: provider.clone(),
                source: SelectionSource::OperatorConfig,
            };
        }
        if let Some(provider) = &session.provider {
            return Self {
                provider: provider.clone(),
                source: SelectionSource::SessionPolicy,
            };
        }
        Self {
            provider: ProviderId::docker(),
            source: SelectionSource::SignedOutDefault,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_out_without_override_defaults_to_docker() {
        assert_eq!(
            RuntimeSelection::resolve(
                &OperatorRuntimeConfig::default(),
                &SessionRuntimePolicy::default()
            ),
            RuntimeSelection {
                provider: ProviderId::docker(),
                source: SelectionSource::SignedOutDefault,
            }
        );
    }

    #[test]
    fn operator_override_precedes_session_policy() {
        let operator = OperatorRuntimeConfig {
            provider: Some(ProviderId::parse("a3s-box").unwrap()),
        };
        let session = SessionRuntimePolicy {
            provider: Some(ProviderId::parse("os-runtime").unwrap()),
        };
        let selected = RuntimeSelection::resolve(&operator, &session);
        assert_eq!(selected.provider.as_str(), "a3s-box");
        assert_eq!(selected.source, SelectionSource::OperatorConfig);
    }

    #[test]
    fn provider_ids_are_closed_and_portable() {
        for invalid in ["", "Docker", "a3s_box", "-docker", "docker-", "a/b"] {
            assert!(ProviderId::parse(invalid).is_err(), "accepted {invalid:?}");
        }
        assert_eq!(
            ProviderId::parse("vendor-runtime-2").unwrap().as_str(),
            "vendor-runtime-2"
        );
        assert!(serde_json::from_str::<RuntimeSelection>(
            r#"{"provider":"Docker","source":"operator_config"}"#
        )
        .is_err());
    }
}
