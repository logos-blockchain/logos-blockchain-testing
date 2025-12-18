use std::time::Duration;

use nomos_libp2p::{
    IdentifySettings, KademliaSettings, Multiaddr, NatSettings, ed25519, gossipsub,
};
use nomos_node::config::network::serde::{BackendSettings, Config, SwarmConfig};
use nomos_utils::net::get_available_udp_port;
use testing_framework_env as tf_env;
use thiserror::Error;

use crate::node_address_from_port;

const PEER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default, Clone)]
pub enum Libp2pNetworkLayout {
    #[default]
    Star,
    Chain,
    Full,
}

#[derive(Default, Clone)]
pub struct NetworkParams {
    pub libp2p_network_layout: Libp2pNetworkLayout,
}

pub type GeneralNetworkConfig = Config;

#[derive(Debug, Error)]
pub enum NetworkConfigError {
    #[error("failed to allocate a free UDP port for libp2p swarm")]
    PortAllocationFailed,
}

fn default_swarm_config() -> SwarmConfig {
    SwarmConfig {
        host: std::net::Ipv4Addr::UNSPECIFIED,
        port: 60000,
        node_key: ed25519::SecretKey::generate(),
        gossipsub_config: gossipsub::Config::default(),
        kademlia_config: KademliaSettings::default(),
        identify_config: IdentifySettings::default(),
        chain_sync_config: cryptarchia_sync::Config::default(),
        nat_config: NatSettings::default(),
    }
}

fn nat_settings(port: u16) -> NatSettings {
    if tf_env::nomos_use_autonat() {
        return NatSettings::default();
    }

    let addr: Multiaddr = format!("/ip4/127.0.0.1/udp/{port}/quic-v1")
        .parse()
        .expect("failed to build loopback multiaddr");
    NatSettings::Static {
        external_address: addr,
    }
}

#[must_use]
pub fn create_network_configs(
    ids: &[[u8; 32]],
    network_params: &NetworkParams,
) -> Result<Vec<GeneralNetworkConfig>, NetworkConfigError> {
    let swarm_configs: Vec<SwarmConfig> = ids
        .iter()
        .map(|id| {
            let mut node_key_bytes = *id;
            let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
                .expect("Failed to generate secret key from bytes");

            let port = get_available_udp_port().ok_or(NetworkConfigError::PortAllocationFailed)?;
            Ok(SwarmConfig {
                node_key,
                port,
                chain_sync_config: cryptarchia_sync::Config {
                    peer_response_timeout: PEER_RESPONSE_TIMEOUT,
                },
                nat_config: nat_settings(port),
                ..default_swarm_config()
            })
        })
        .collect::<Result<_, _>>()?;

    let all_initial_peers = initial_peers_by_network_layout(&swarm_configs, network_params);

    Ok(swarm_configs
        .iter()
        .zip(all_initial_peers)
        .map(|(swarm_config, initial_peers)| GeneralNetworkConfig {
            backend: BackendSettings {
                initial_peers,
                swarm: swarm_config.to_owned(),
            },
        })
        .collect())
}

fn initial_peers_by_network_layout(
    swarm_configs: &[SwarmConfig],
    network_params: &NetworkParams,
) -> Vec<Vec<Multiaddr>> {
    if swarm_configs.is_empty() {
        return Vec::new();
    }

    let mut all_initial_peers = vec![];

    match network_params.libp2p_network_layout {
        Libp2pNetworkLayout::Star => {
            // First node is the hub - has no initial peers
            all_initial_peers.push(vec![]);
            let first_addr = node_address_from_port(swarm_configs[0].port);

            // All other nodes connect to the first node
            for _ in 1..swarm_configs.len() {
                all_initial_peers.push(vec![first_addr.clone()]);
            }
        }
        Libp2pNetworkLayout::Chain => {
            // First node has no initial peers
            all_initial_peers.push(vec![]);

            // Each subsequent node connects to the previous one
            for i in 1..swarm_configs.len() {
                let prev_addr = node_address_from_port(swarm_configs[i - 1].port);
                all_initial_peers.push(vec![prev_addr]);
            }
        }
        Libp2pNetworkLayout::Full => {
            // Each node connects to all previous nodes, unidirectional connections
            for i in 0..swarm_configs.len() {
                let mut peers = vec![];
                for swarm_config in swarm_configs.iter().take(i) {
                    peers.push(node_address_from_port(swarm_config.port));
                }
                all_initial_peers.push(peers);
            }
        }
    }

    all_initial_peers
}
