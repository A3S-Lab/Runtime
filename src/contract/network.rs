use serde::{Deserialize, Serialize};
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use super::observation::RuntimeObservation;

const SERVICE_ENDPOINT_CLAIM_PREFIX: &str = "a3s.runtime.service-endpoint.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    None,
    Outbound,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    Tcp,
    Udp,
}

impl TransportProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePort {
    pub name: String,
    pub container_port: u16,
    pub protocol: TransportProtocol,
}

impl RuntimePort {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_service_port_name(&self.name)?;
        if self.container_port == 0 {
            return Err("container_port must be positive".into());
        }
        Ok(())
    }
}

/// One provider-published, generation-bound loopback endpoint for a declared
/// Runtime Service port.
///
/// The value is encoded inside the existing Runtime evidence claim map so the
/// endpoint remains bound to the observation's provider build and spec digest.
/// Providers own endpoint lifecycle; callers consume this type instead of
/// defining product-specific claim prefixes or endpoint registries.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeServiceEndpoint {
    pub port_name: String,
    pub protocol: TransportProtocol,
    pub address: IpAddr,
    pub port: u16,
}

impl RuntimeServiceEndpoint {
    pub fn new(
        port_name: impl Into<String>,
        protocol: TransportProtocol,
        address: IpAddr,
        port: u16,
    ) -> Result<Self, String> {
        let endpoint = Self {
            port_name: port_name.into(),
            protocol,
            address,
            port,
        };
        endpoint.validate()?;
        Ok(endpoint)
    }

    pub fn node_local_tcp(port_name: impl Into<String>, port: u16) -> Result<Self, String> {
        Self::new(
            port_name,
            TransportProtocol::Tcp,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        )
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_service_port_name(&self.port_name)?;
        if !self.address.is_loopback() || self.port == 0 {
            return Err(
                "Runtime service endpoint must use an explicit positive loopback socket".into(),
            );
        }
        Ok(())
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.address, self.port)
    }

    pub fn claim_key(&self) -> String {
        format!("{SERVICE_ENDPOINT_CLAIM_PREFIX}{}", self.port_name)
    }

    pub fn claim_value(&self) -> String {
        format!("{}://{}", self.protocol.as_str(), self.socket_addr())
    }

    pub fn insert_claim(&self, claims: &mut BTreeMap<String, String>) -> Result<(), String> {
        self.validate()?;
        let key = self.claim_key();
        let value = self.claim_value();
        match claims.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(value);
                Ok(())
            }
            Entry::Occupied(entry) if entry.get() == &value => Ok(()),
            Entry::Occupied(_) => Err(format!(
                "Runtime evidence contains conflicting service endpoint {:?}",
                self.port_name
            )),
        }
    }

    pub fn from_observation(
        observation: &RuntimeObservation,
        port_name: &str,
    ) -> Result<Self, String> {
        observation.validate()?;
        validate_service_port_name(port_name)?;
        let key = format!("{SERVICE_ENDPOINT_CLAIM_PREFIX}{port_name}");
        let value = observation
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.claims.get(&key))
            .ok_or_else(|| {
                format!("Runtime observation has no service endpoint for port {port_name:?}")
            })?;
        Self::from_claim(port_name, value)
    }

    pub(crate) fn from_claims(claims: &BTreeMap<String, String>) -> Result<Vec<Self>, String> {
        claims
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(SERVICE_ENDPOINT_CLAIM_PREFIX)
                    .map(|port_name| Self::from_claim(port_name, value))
            })
            .collect()
    }

    pub(crate) fn remove_claims(claims: &mut BTreeMap<String, String>) {
        claims.retain(|key, _| !key.starts_with(SERVICE_ENDPOINT_CLAIM_PREFIX));
    }

    fn from_claim(port_name: &str, value: &str) -> Result<Self, String> {
        let (protocol, socket) = if let Some(socket) = value.strip_prefix("tcp://") {
            (TransportProtocol::Tcp, socket)
        } else if let Some(socket) = value.strip_prefix("udp://") {
            (TransportProtocol::Udp, socket)
        } else {
            return Err("Runtime service endpoint claim has an unsupported protocol".into());
        };
        let socket = socket
            .parse::<SocketAddr>()
            .map_err(|error| format!("Runtime service endpoint claim is invalid: {error}"))?;
        let endpoint = Self::new(port_name, protocol, socket.ip(), socket.port())?;
        if endpoint.claim_value() != value {
            return Err("Runtime service endpoint claim is not canonical".into());
        }
        Ok(endpoint)
    }
}

fn validate_service_port_name(value: &str) -> Result<(), String> {
    super::validate_name("service port name", value)?;
    if SERVICE_ENDPOINT_CLAIM_PREFIX.len() + value.len() > 255 {
        return Err("Runtime service port name exceeds the endpoint evidence key bound".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeNetworkSpec {
    pub mode: NetworkMode,
    pub ports: Vec<RuntimePort>,
}

impl RuntimeNetworkSpec {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.ports.len() > 64 {
            return Err("Runtime unit declares more than 64 ports".into());
        }
        if self.mode != NetworkMode::Service && !self.ports.is_empty() {
            return Err("declared ports require service network mode".into());
        }
        let mut names = BTreeSet::new();
        let mut sockets = BTreeSet::new();
        for port in &self.ports {
            port.validate()?;
            if !names.insert(&port.name) {
                return Err(format!("duplicate Runtime port name {:?}", port.name));
            }
            if !sockets.insert((port.container_port, port.protocol)) {
                return Err(format!(
                    "duplicate Runtime port socket {}/{}",
                    port.container_port,
                    match port.protocol {
                        TransportProtocol::Tcp => "tcp",
                        TransportProtocol::Udp => "udp",
                    }
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn has_port(&self, name: &str) -> bool {
        self.ports.iter().any(|port| port.name == name)
    }
}
