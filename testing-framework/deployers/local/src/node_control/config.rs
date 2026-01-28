use std::collections::HashMap;

use nomos_libp2p::Multiaddr;
use nomos_utils::net::get_available_udp_port;
use rand::Rng as _;
use testing_framework_config::topology::configs::{
    consensus,
    runtime::{build_general_config_for_node, build_initial_peers},
    time::GeneralTimeConfig,
};
use testing_framework_core::{
    scenario::{PeerSelection, StartNodeOptions},
    topology::{
        config::{NodeConfigPatch, TopologyConfig},
        configs::GeneralConfig,
        generation::{GeneratedNodeConfig, GeneratedTopology},
    },
};

use super::LocalNodeManagerError;

pub(super) fn build_general_config_for(
    descriptors: &GeneratedTopology,
    base_consensus: &consensus::GeneralConsensusConfig,
    base_time: &GeneralTimeConfig,
    index: usize,
    peer_ports_by_name: &HashMap<String, u16>,
    options: &StartNodeOptions,
    peer_ports: &[u16],
) -> Result<(GeneralConfig, u16, Option<NodeConfigPatch>), LocalNodeManagerError> {
    if let Some(node) = descriptor_for(descriptors, index) {
        let mut config = node.general.clone();
        let initial_peers = resolve_initial_peers(
            peer_ports_by_name,
            options,
            &config.network_config.backend.initial_peers,
            descriptors,
            peer_ports,
        )?;

        config.network_config.backend.initial_peers = initial_peers;

        return Ok((config, node.network_port(), node.config_patch.clone()));
    }

    let id = random_node_id();
    let network_port = allocate_udp_port("network port")?;
    let blend_port = allocate_udp_port("Blend port")?;
    let topology = descriptors.config();
    let initial_peers =
        resolve_initial_peers(peer_ports_by_name, options, &[], descriptors, peer_ports)?;
    let general_config = build_general_config_for_node(
        id,
        network_port,
        initial_peers,
        blend_port,
        &topology.consensus_params,
        &topology.wallet_config,
        base_consensus,
        base_time,
    )
    .map_err(|source| LocalNodeManagerError::Config { source })?;

    Ok((general_config, network_port, None))
}

fn descriptor_for(descriptors: &GeneratedTopology, index: usize) -> Option<&GeneratedNodeConfig> {
    descriptors.nodes().get(index)
}

fn resolve_peer_names(
    peer_ports_by_name: &HashMap<String, u16>,
    peer_names: &[String],
) -> Result<Vec<Multiaddr>, LocalNodeManagerError> {
    let mut peers = Vec::with_capacity(peer_names.len());
    for name in peer_names {
        let port =
            peer_ports_by_name
                .get(name)
                .ok_or_else(|| LocalNodeManagerError::InvalidArgument {
                    message: format!("unknown peer name '{name}'"),
                })?;
        peers.push(testing_framework_config::node_address_from_port(*port));
    }
    Ok(peers)
}

fn resolve_initial_peers(
    peer_ports_by_name: &HashMap<String, u16>,
    options: &StartNodeOptions,
    default_peers: &[Multiaddr],
    descriptors: &GeneratedTopology,
    peer_ports: &[u16],
) -> Result<Vec<Multiaddr>, LocalNodeManagerError> {
    match &options.peers {
        PeerSelection::Named(names) => resolve_peer_names(peer_ports_by_name, names),
        PeerSelection::DefaultLayout => {
            if !default_peers.is_empty() {
                Ok(default_peers.to_vec())
            } else {
                let topology: &TopologyConfig = descriptors.config();
                Ok(build_initial_peers(&topology.network_params, peer_ports))
            }
        }
        PeerSelection::None => Ok(Vec::new()),
    }
}

fn random_node_id() -> [u8; 32] {
    let mut id = [0u8; 32];
    rand::thread_rng().fill(&mut id);
    id
}

fn allocate_udp_port(label: &'static str) -> Result<u16, LocalNodeManagerError> {
    get_available_udp_port().ok_or_else(|| LocalNodeManagerError::PortAllocation {
        message: format!("failed to allocate free UDP port for {label}"),
    })
}
