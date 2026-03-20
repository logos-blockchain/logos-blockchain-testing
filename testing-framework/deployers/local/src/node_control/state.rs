use std::collections::HashMap;

use crate::env::{LocalDeployerEnv, Node};

pub(crate) struct LocalNodeManagerState<E: LocalDeployerEnv> {
    pub(crate) node_count: usize,
    pub(crate) peer_ports: Vec<u16>,
    pub(crate) peer_ports_by_name: HashMap<String, u16>,
    pub(crate) clients_by_name: HashMap<String, E::NodeClient>,
    pub(crate) indices_by_name: HashMap<String, usize>,
    pub(crate) nodes: Vec<Node<E>>,
    pub(crate) template_config: Option<E::NodeConfig>,
}

impl<E: LocalDeployerEnv> LocalNodeManagerState<E> {
    fn register_common(&mut self, node_name: &str, network_port: u16, client: E::NodeClient) {
        self.peer_ports.push(network_port);
        self.peer_ports_by_name
            .insert(node_name.to_string(), network_port);
        self.clients_by_name.insert(node_name.to_string(), client);
    }

    pub(super) fn register_node(
        &mut self,
        node_name: &str,
        network_port: u16,
        client: E::NodeClient,
        node: Node<E>,
    ) {
        self.register_common(node_name, network_port, client);
        let index = self.nodes.len();
        self.indices_by_name.insert(node_name.to_string(), index);
        self.node_count += 1;
        self.nodes.push(node);
    }
}
