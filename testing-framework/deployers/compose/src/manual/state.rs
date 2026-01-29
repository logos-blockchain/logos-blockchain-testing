use std::collections::{HashMap, HashSet};

use testing_framework_core::nodes::ApiClient;

use super::ManualClusterError;
use crate::infrastructure::ports::NodeHostPorts;

pub struct ManualClusterState {
    pub started_indices: HashSet<usize>,
    pub clients_by_name: HashMap<String, ApiClient>,
    pub name_to_index: HashMap<String, usize>,
    pub clients_by_index: Vec<Option<ApiClient>>,
    pub ports_by_index: Vec<Option<NodeHostPorts>>,
}

impl ManualClusterState {
    pub fn new(total_nodes: usize) -> Self {
        Self {
            started_indices: HashSet::new(),
            clients_by_name: HashMap::new(),
            name_to_index: HashMap::new(),
            clients_by_index: vec![None; total_nodes],
            ports_by_index: vec![None; total_nodes],
        }
    }

    pub fn reset(&mut self) {
        self.started_indices.clear();
        self.clients_by_name.clear();
        self.name_to_index.clear();

        for slot in &mut self.clients_by_index {
            *slot = None;
        }
        for slot in &mut self.ports_by_index {
            *slot = None;
        }
    }

    pub fn register_node(
        &mut self,
        index: usize,
        label: String,
        client: ApiClient,
        ports: NodeHostPorts,
    ) -> Result<(), ManualClusterError> {
        if self.clients_by_name.contains_key(&label) {
            return Err(ManualClusterError::NameExists { name: label });
        }
        if self.started_indices.contains(&index) {
            return Err(ManualClusterError::AlreadyStarted { index });
        }

        self.started_indices.insert(index);
        self.name_to_index.insert(label.clone(), index);
        self.clients_by_name.insert(label, client.clone());
        if let Some(slot) = self.clients_by_index.get_mut(index) {
            *slot = Some(client);
        }
        if let Some(slot) = self.ports_by_index.get_mut(index) {
            *slot = Some(ports);
        }

        Ok(())
    }
}

pub fn normalize_label(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("node-") {
        Some(trimmed.to_string())
    } else {
        Some(format!("node-{trimmed}"))
    }
}

pub fn next_available_index(started: &HashSet<usize>, total_nodes: usize) -> Option<usize> {
    (0..total_nodes).find(|index| !started.contains(index))
}
