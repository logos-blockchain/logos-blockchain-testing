use std::{path, sync::Mutex};

use async_trait::async_trait;
use path::{Path, PathBuf};
use testing_framework_core::{
    manual::ManualClusterHandle,
    nodes::ApiClient,
    scenario::{CleanupGuard, DynError, PeerSelection, StartNodeOptions, StartedNode},
    topology::{
        config::{TopologyBuildError, TopologyBuilder, TopologyConfig},
        generation::GeneratedTopology,
        readiness::{ManualNetworkReadiness, ReadinessCheck, ReadinessError, ReadinessNode},
    },
};
use thiserror::Error;

use crate::{
    docker::{commands::compose_up_service, ensure_docker_available},
    errors::ComposeRunnerError,
    infrastructure::{
        environment::{StackEnvironment, ensure_supported_topology, prepare_environment_manual},
        ports::{NodeHostPorts, compose_runner_host, node_identifier, resolve_service_port_with},
    },
};

mod network;
mod readiness;
mod state;

use network::api_client_from_host_ports;
use readiness::readiness_nodes;
use state::{ManualClusterState, next_available_index, normalize_label};

#[derive(Debug, Error)]
pub enum ManualClusterError {
    #[error("failed to build topology: {source}")]
    Build {
        #[source]
        source: TopologyBuildError,
    },
    #[error(transparent)]
    Compose(#[from] ComposeRunnerError),
    #[error("manual compose cluster only supports default peer selection")]
    UnsupportedPeers,
    #[error("manual compose cluster does not support config patches")]
    UnsupportedConfigPatch,
    #[error("cluster has already been stopped")]
    Stopped,
    #[error("node name '{name}' already exists")]
    NameExists { name: String },
    #[error("node index {index} is out of range (max {max})")]
    IndexOutOfRange { index: usize, max: usize },
    #[error("node index {index} already started")]
    AlreadyStarted { index: usize },
    #[error("no available nodes to start")]
    NoAvailableNodes,
    #[error("node name cannot be empty")]
    EmptyName,
}

struct EnvironmentSnapshot {
    compose_path: PathBuf,
    project_name: String,
    root: PathBuf,
}

/// Imperative, compose-backed cluster that can start nodes on demand.
pub struct ComposeManualCluster {
    descriptors: GeneratedTopology,
    environment: Mutex<Option<StackEnvironment>>,
    state: Mutex<ManualClusterState>,
    host: String,
}

impl ComposeManualCluster {
    pub(crate) async fn from_config(config: TopologyConfig) -> Result<Self, ManualClusterError> {
        let builder = TopologyBuilder::new(config);
        let descriptors = builder
            .build()
            .map_err(|source| ManualClusterError::Build { source })?;

        ensure_supported_topology(&descriptors)?;

        ensure_docker_available().await?;
        let environment = prepare_environment_manual(&descriptors, None).await?;

        let total_nodes = descriptors.nodes().len();

        Ok(Self {
            descriptors,
            environment: Mutex::new(Some(environment)),
            state: Mutex::new(ManualClusterState::new(total_nodes)),
            host: compose_runner_host(),
        })
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, ManualClusterState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn resolve_label_and_index(
        &self,
        name: &str,
        total_nodes: usize,
    ) -> Result<(usize, String), ManualClusterError> {
        let state = self.lock_state();

        let label = normalize_label(name).ok_or(ManualClusterError::EmptyName)?;
        let index = match state.name_to_index.get(&label).copied() {
            Some(index) => index,
            None => next_available_index(&state.started_indices, total_nodes)
                .ok_or(ManualClusterError::NoAvailableNodes)?,
        };

        let node_label = label;

        if state.clients_by_name.contains_key(&node_label) {
            return Err(ManualClusterError::NameExists { name: node_label });
        }

        if state.started_indices.contains(&index) {
            return Err(ManualClusterError::AlreadyStarted { index });
        }

        Ok((index, node_label))
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<ApiClient> {
        if name.trim().is_empty() {
            return None;
        }

        let state = self.lock_state();

        normalize_label(name).and_then(|label| state.clients_by_name.get(&label).cloned())
    }

    pub async fn start_node(&self, name: &str) -> Result<StartedNode, ManualClusterError> {
        self.start_node_with(name, StartNodeOptions::default())
            .await
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, ManualClusterError> {
        if !matches!(options.peers, PeerSelection::DefaultLayout) {
            return Err(ManualClusterError::UnsupportedPeers);
        }

        if options.config_patch.is_some() {
            return Err(ManualClusterError::UnsupportedConfigPatch);
        }

        let total_nodes = self.descriptors.nodes().len();
        let (index, node_label) = self.resolve_label_and_index(name, total_nodes)?;

        let snapshot = self.environment_snapshot()?;
        let service = node_identifier(index);

        compose_up_service(
            &snapshot.compose_path,
            &snapshot.project_name,
            &snapshot.root,
            &service,
        )
        .await
        .map_err(ComposeRunnerError::Compose)?;

        let ports = discover_node_ports(
            &snapshot.compose_path,
            &snapshot.project_name,
            &snapshot.root,
            &service,
            &self.descriptors,
            index,
        )
        .await?;

        let client = api_client_from_host_ports(&ports, &self.host)?;

        let mut state = self.lock_state();

        state.register_node(index, node_label.clone(), client.clone(), ports)?;

        Ok(StartedNode {
            name: node_label,
            api: client,
        })
    }

    pub fn stop_all(&self) {
        let mut env = self
            .environment
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Some(environment) = env.take() else {
            return;
        };

        if let Ok(cleanup) = environment.into_cleanup() {
            Box::new(cleanup).cleanup();
        }

        let mut state = self.lock_state();
        state.reset();
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        let nodes = self.readiness_nodes();
        if nodes.len() <= 1 {
            return Ok(());
        }

        ManualNetworkReadiness::new(nodes).wait().await
    }

    fn readiness_nodes(&self) -> Vec<ReadinessNode> {
        let state = self.lock_state();
        readiness_nodes(&state, &self.descriptors)
    }

    fn environment_snapshot(&self) -> Result<EnvironmentSnapshot, ManualClusterError> {
        let env = self
            .environment
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Some(environment) = env.as_ref() else {
            return Err(ManualClusterError::Stopped);
        };

        Ok(EnvironmentSnapshot {
            compose_path: environment.compose_path().to_path_buf(),
            project_name: environment.project_name().to_owned(),
            root: environment.root().to_path_buf(),
        })
    }
}

impl Drop for ComposeManualCluster {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[async_trait]
impl ManualClusterHandle for ComposeManualCluster {
    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        self.start_node_with(name, options)
            .await
            .map_err(|err| err.into())
    }

    async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.wait_network_ready().await.map_err(|err| err.into())
    }
}

async fn discover_node_ports(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    service: &str,
    descriptors: &GeneratedTopology,
    index: usize,
) -> Result<NodeHostPorts, ManualClusterError> {
    let node =
        descriptors
            .nodes()
            .get(index)
            .ok_or_else(|| ManualClusterError::IndexOutOfRange {
                index,
                max: descriptors.nodes().len().saturating_sub(1),
            })?;

    let api = resolve_service_port_with(compose_path, project_name, root, service, node.api_port())
        .await?;

    let testing = resolve_service_port_with(
        compose_path,
        project_name,
        root,
        service,
        node.testing_http_port(),
    )
    .await?;

    Ok(NodeHostPorts { api, testing })
}
