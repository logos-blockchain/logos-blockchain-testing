use std::collections::HashMap;

use testing_framework_core::nodes::{ApiClient, validator::Validator};

pub(crate) struct LocalDynamicState {
    pub(crate) validator_count: usize,
    pub(crate) peer_ports: Vec<u16>,
    pub(crate) peer_ports_by_name: HashMap<String, u16>,
    pub(crate) clients_by_name: HashMap<String, ApiClient>,
    pub(crate) validators: Vec<Validator>,
}

impl LocalDynamicState {
    fn register_common(&mut self, node_name: &str, network_port: u16, client: ApiClient) {
        self.peer_ports.push(network_port);
        self.peer_ports_by_name
            .insert(node_name.to_string(), network_port);
        self.clients_by_name.insert(node_name.to_string(), client);
    }

    pub(super) fn register_validator(
        &mut self,
        node_name: &str,
        network_port: u16,
        client: ApiClient,
        node: Validator,
    ) {
        self.register_common(node_name, network_port, client);
        self.validator_count += 1;
        self.validators.push(node);
    }
}
