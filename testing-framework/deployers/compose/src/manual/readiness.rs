use std::collections::HashSet;

use testing_framework_core::topology::{
    generation::GeneratedTopology,
    readiness::{ReadinessNode, build_readiness_nodes},
    utils::multiaddr_port,
};

use super::state::ManualClusterState;

pub fn readiness_nodes(
    state: &ManualClusterState,
    descriptors: &GeneratedTopology,
) -> Vec<ReadinessNode> {
    let mut indices = state.started_indices.iter().copied().collect::<Vec<_>>();
    indices.sort_unstable();

    let iter = indices.into_iter().filter_map(|index| {
        let api = state.clients_by_index.get(index).and_then(|c| c.clone())?;
        let node = descriptors.nodes().get(index)?;
        let initial_peers = node
            .general
            .network_config
            .backend
            .initial_peers
            .iter()
            .filter_map(multiaddr_port)
            .collect::<HashSet<_>>();

        Some((
            format!("node#{index}@{}", node.network_port()),
            api,
            node.network_port(),
            initial_peers,
        ))
    });

    build_readiness_nodes(iter)
}
