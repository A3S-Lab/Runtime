use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePort {
    pub name: String,
    pub container_port: u16,
    pub protocol: TransportProtocol,
}

impl RuntimePort {
    pub(crate) fn validate(&self) -> Result<(), String> {
        super::validate_name("port name", &self.name)?;
        if self.container_port == 0 {
            return Err("container_port must be positive".into());
        }
        Ok(())
    }
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
