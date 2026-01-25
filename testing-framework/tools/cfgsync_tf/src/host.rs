use std::net::Ipv4Addr;

use testing_framework_config::constants::{
    DEFAULT_API_PORT, DEFAULT_BLEND_NETWORK_PORT, DEFAULT_LIBP2P_NETWORK_PORT,
};

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub enum HostKind {
    Validator,
}

#[derive(Eq, PartialEq, Hash, Clone)]
pub struct Host {
    pub kind: HostKind,
    pub ip: Ipv4Addr,
    pub identifier: String,
    pub network_port: u16,
    pub blend_port: u16,
    pub api_port: u16,
    pub testing_http_port: u16,
}

#[derive(Clone, Copy)]
pub struct PortOverrides {
    pub network_port: Option<u16>,
    pub blend_port: Option<u16>,
    pub api_port: Option<u16>,
    pub testing_http_port: Option<u16>,
}

impl Host {
    fn from_parts(kind: HostKind, ip: Ipv4Addr, identifier: String, ports: PortOverrides) -> Self {
        Self {
            kind,
            ip,
            identifier,
            network_port: ports.network_port.unwrap_or(DEFAULT_LIBP2P_NETWORK_PORT),
            blend_port: ports.blend_port.unwrap_or(DEFAULT_BLEND_NETWORK_PORT),
            api_port: ports.api_port.unwrap_or(DEFAULT_API_PORT),
            testing_http_port: ports.testing_http_port.unwrap_or(DEFAULT_API_PORT + 1),
        }
    }

    #[must_use]
    pub fn validator_from_ip(ip: Ipv4Addr, identifier: String, ports: PortOverrides) -> Self {
        Self::from_parts(HostKind::Validator, ip, identifier, ports)
    }
}

#[must_use]
pub fn sort_hosts(mut hosts: Vec<Host>) -> Vec<Host> {
    hosts.sort_by_key(|host| {
        let index = host
            .identifier
            .rsplit('-')
            .next()
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(0);
        let kind = match host.kind {
            HostKind::Validator => 0,
        };
        (kind, index)
    });
    hosts
}
