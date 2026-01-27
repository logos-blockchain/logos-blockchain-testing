use std::collections::HashMap;

use testing_framework_core::nodes::{ApiClient, node::Node};

pub(crate) struct LocalNodeManagerState {
    pub(crate) node_count: usize,
    pub(crate) peer_ports: Vec<u16>,
    pub(crate) peer_ports_by_name: HashMap<String, u16>,
    pub(crate) clients_by_name: HashMap<String, ApiClient>,
    pub(crate) nodes: Vec<Node>,
}

impl LocalNodeManagerState {
    fn register_common(&mut self, node_name: &str, network_port: u16, client: ApiClient) {
        self.peer_ports.push(network_port);
        self.peer_ports_by_name
            .insert(node_name.to_string(), network_port);
        self.clients_by_name.insert(node_name.to_string(), client);
    }

    pub(super) fn register_node(
        &mut self,
        node_name: &str,
        network_port: u16,
        client: ApiClient,
        node: Node,
    ) {
        self.register_common(node_name, network_port, client);
        self.node_count += 1;
        self.nodes.push(node);
    }
}
