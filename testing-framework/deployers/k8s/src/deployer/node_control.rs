use std::{collections::HashSet, sync::Arc};

use kube::Client;
use testing_framework_core::scenario::{
    DynError, HttpReadinessRequirement, NodeControlHandle, PeerSelection, StartNodeOptions,
    StartedNode,
};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{
    env::{K8sDeployEnv, node_deployment_name, node_role, wait_for_node_http},
    manual::{canonical_node_name, parse_node_index, patch_node_replicas, wait_for_replicas},
    wait::{PortForwardRegistry, http_poll_interval, node_http_timeout},
};

/// Node lifecycle control for a framework-managed k8s deployment.
///
/// Nodes are stopped and started by patching their per-node deployment
/// replicas, mirroring the `ManualCluster` primitives. Nodes are addressed by
/// their canonical `node-<index>` names. When the cluster is reached through
/// `kubectl port-forward` fallback, lifecycle operations respawn the node's
/// forwards on their original local ports so existing clients keep working.
/// Config overrides and persist/snapshot directories are not supported; use
/// `ManualCluster` for cfgsync-backed overrides.
pub struct K8sNodeControl<E: K8sDeployEnv> {
    client: Client,
    namespace: String,
    release: String,
    node_host: String,
    node_api_ports: Vec<u16>,
    node_auxiliary_ports: Vec<u16>,
    forwards: PortForwardRegistry,
    running: Mutex<HashSet<usize>>,
    operations: Vec<Mutex<()>>,
    _marker: std::marker::PhantomData<E>,
}

#[derive(Debug, Error)]
/// Failures while controlling nodes of a managed k8s deployment.
pub enum K8sNodeControlError {
    #[error("invalid node name '{name}'; expected node-<index>")]
    /// The node name did not match the canonical `node-<index>` form.
    InvalidNodeName { name: String },
    #[error("node index {index} is out of range for topology with {nodes} nodes")]
    /// The node index exceeds the deployed topology.
    NodeIndexOutOfRange { index: usize, nodes: usize },
    #[error("node '{name}' is already running")]
    /// Start was requested for a node that is running.
    NodeAlreadyRunning { name: String },
    #[error("node '{name}' is not running")]
    /// Restart/stop was requested for a node that is not running.
    NodeNotRunning { name: String },
    #[error("unsupported start options for managed k8s node control: {message}")]
    /// The provided start options cannot be honored by this handle.
    UnsupportedStartOptions { message: String },
    #[error("failed to scale node '{name}': {source}")]
    /// Patching the node deployment replicas failed.
    Scale {
        name: String,
        #[source]
        source: DynError,
    },
    #[error("node readiness failed for '{name}': {source}")]
    /// The node did not become ready after starting.
    NodeReadiness {
        name: String,
        #[source]
        source: DynError,
    },
    #[error("failed to build node client for '{name}': {source}")]
    /// Rebuilding the node API client failed.
    NodeClient {
        name: String,
        #[source]
        source: DynError,
    },
    #[error("failed to re-establish port-forwards for '{name}': {source}")]
    /// Respawning the node's port-forwards after a lifecycle operation failed.
    Forward {
        name: String,
        #[source]
        source: DynError,
    },
}

impl<E: K8sDeployEnv> K8sNodeControl<E> {
    /// Creates a control handle over the per-node deployments of a managed
    /// cluster; all nodes are assumed to be running.
    pub(crate) fn new(
        client: Client,
        namespace: String,
        release: String,
        node_host: String,
        node_api_ports: Vec<u16>,
        node_auxiliary_ports: Vec<u16>,
        forwards: PortForwardRegistry,
    ) -> Self {
        let running = (0..node_api_ports.len()).collect();
        let operations = (0..node_api_ports.len()).map(|_| Mutex::new(())).collect();
        Self {
            client,
            namespace,
            release,
            node_host,
            node_api_ports,
            node_auxiliary_ports,
            forwards,
            running: Mutex::new(running),
            operations,
            _marker: std::marker::PhantomData,
        }
    }

    /// Respawns the node's port-forwards on their original local ports after
    /// its pod was replaced. No-op in NodePort mode.
    async fn refresh_forwards(&self, index: usize) -> Result<(), K8sNodeControlError> {
        if self.forwards.is_empty() {
            return Ok(());
        }

        let forwards = self.forwards.clone();
        tokio::task::spawn_blocking(move || forwards.respawn_node(index))
            .await
            .map_err(|source| K8sNodeControlError::Forward {
                name: canonical_node_name(index),
                source: source.into(),
            })?
            .map_err(|source| K8sNodeControlError::Forward {
                name: canonical_node_name(index),
                source: source.into(),
            })
    }

    /// Returns the number of nodes in the deployed topology.
    fn node_count(&self) -> usize {
        self.node_api_ports.len()
    }

    /// Resolves a canonical `node-<index>` name into an in-range index.
    fn require_node_index(&self, name: &str) -> Result<usize, K8sNodeControlError> {
        let index = parse_node_index(name).ok_or_else(|| K8sNodeControlError::InvalidNodeName {
            name: name.to_owned(),
        })?;
        if index >= self.node_count() {
            return Err(K8sNodeControlError::NodeIndexOutOfRange {
                index,
                nodes: self.node_count(),
            });
        }
        Ok(index)
    }

    /// Patches the node deployment to the requested replica count.
    async fn patch_replicas(&self, index: usize, replicas: i32) -> Result<(), K8sNodeControlError> {
        patch_node_replicas(
            &self.client,
            &self.namespace,
            &node_deployment_name::<E>(&self.release, index),
            replicas,
        )
        .await
        .map_err(|source| K8sNodeControlError::Scale {
            name: canonical_node_name(index),
            source: source.into(),
        })
    }

    /// Waits for a patched node deployment to reach the requested replicas.
    async fn wait_for_replicas(
        &self,
        index: usize,
        replicas: i32,
    ) -> Result<(), K8sNodeControlError> {
        let node_name = canonical_node_name(index);
        wait_for_replicas(
            &self.client,
            &self.namespace,
            &node_deployment_name::<E>(&self.release, index),
            &node_name,
            replicas,
        )
        .await
        .map_err(|source| K8sNodeControlError::Scale {
            name: node_name,
            source: source.into(),
        })
    }

    async fn set_running(&self, index: usize, is_running: bool) {
        let mut running = self.running.lock().await;
        if is_running {
            running.insert(index);
        } else {
            running.remove(&index);
        }
    }

    /// Probes the node readiness endpoint until it reports healthy.
    async fn wait_node_ready_by_index(&self, index: usize) -> Result<(), K8sNodeControlError> {
        let port = self.node_api_ports[index];
        wait_for_node_http::<E>(
            &[port],
            node_role::<E>(),
            &self.node_host,
            node_http_timeout(),
            http_poll_interval(),
            HttpReadinessRequirement::AllNodesReady,
        )
        .await
        .map_err(|source| K8sNodeControlError::NodeReadiness {
            name: canonical_node_name(index),
            source: source.into(),
        })
    }

    /// Rebuilds the node API client from the discovered host ports.
    fn build_client(&self, index: usize) -> Result<E::NodeClient, K8sNodeControlError> {
        E::node_client_from_ports(
            &self.node_host,
            self.node_api_ports[index],
            self.node_auxiliary_ports[index],
        )
        .map_err(|source| K8sNodeControlError::NodeClient {
            name: canonical_node_name(index),
            source,
        })
    }

    /// Restarts a running node by scaling its deployment to zero and back,
    /// then waits for readiness. The running set is updated after each
    /// successful patch so it tracks Kubernetes desired state even when a
    /// readiness wait fails. A per-node lock serializes the full operation,
    /// including port-forward recovery and HTTP readiness.
    async fn restart(&self, name: &str) -> Result<(), K8sNodeControlError> {
        let index = self.require_node_index(name)?;
        let _operation = self.operations[index].lock().await;
        {
            let running = self.running.lock().await;
            ensure_node_running(&running, index)?;
        }

        self.patch_replicas(index, 0).await?;
        self.set_running(index, false).await;
        self.wait_for_replicas(index, 0).await?;
        self.patch_replicas(index, 1).await?;
        self.set_running(index, true).await;
        self.wait_for_replicas(index, 1).await?;
        self.refresh_forwards(index).await?;
        self.wait_node_ready_by_index(index).await
    }

    /// Starts a stopped node, waits for readiness, and returns its client.
    /// The node counts as running once the scale-up succeeds, even if the
    /// subsequent readiness wait fails.
    async fn start(&self, name: &str) -> Result<StartedNode<E>, K8sNodeControlError> {
        let index = self.require_node_index(name)?;
        let _operation = self.operations[index].lock().await;
        if self.running.lock().await.contains(&index) {
            return Err(K8sNodeControlError::NodeAlreadyRunning {
                name: canonical_node_name(index),
            });
        }

        self.patch_replicas(index, 1).await?;
        self.set_running(index, true).await;
        self.wait_for_replicas(index, 1).await?;
        self.refresh_forwards(index).await?;
        self.wait_node_ready_by_index(index).await?;
        let client = self.build_client(index)?;

        Ok(StartedNode {
            name: canonical_node_name(index),
            client,
        })
    }

    /// Stops a running node by scaling its deployment to zero replicas.
    async fn stop(&self, name: &str) -> Result<(), K8sNodeControlError> {
        let index = self.require_node_index(name)?;
        let _operation = self.operations[index].lock().await;
        {
            let running = self.running.lock().await;
            ensure_node_running(&running, index)?;
        }
        self.patch_replicas(index, 0).await?;
        self.set_running(index, false).await;
        self.wait_for_replicas(index, 0).await
    }
}

/// Rejects every non-default start option: this handle can only re-launch a
/// node exactly as it was deployed, so accepting an option it cannot honor
/// would report a configured start that never happened.
fn ensure_default_start_options<E: K8sDeployEnv>(
    options: &StartNodeOptions<E>,
) -> Result<(), K8sNodeControlError> {
    if options.persist_dir.is_some() || options.snapshot_dir.is_some() {
        return Err(K8sNodeControlError::UnsupportedStartOptions {
            message: "persist/snapshot directories are not supported".to_owned(),
        });
    }

    let default_peers = matches!(options.peers, None | Some(PeerSelection::DefaultLayout));
    if !default_peers || options.config_override.is_some() || options.config_patch.is_some() {
        return Err(K8sNodeControlError::UnsupportedStartOptions {
            message: "config overrides are not supported by managed k8s node control; use a \
                      ManualCluster"
                .to_owned(),
        });
    }

    if !options.args.is_empty() {
        return Err(K8sNodeControlError::UnsupportedStartOptions {
            message: "extra process arguments are not supported by managed k8s node control"
                .to_owned(),
        });
    }

    if options.runtime.start_timeout.is_some() {
        return Err(K8sNodeControlError::UnsupportedStartOptions {
            message: "start timeout overrides are not supported by managed k8s node control"
                .to_owned(),
        });
    }

    Ok(())
}

fn ensure_node_running(running: &HashSet<usize>, index: usize) -> Result<(), K8sNodeControlError> {
    if running.contains(&index) {
        return Ok(());
    }

    Err(K8sNodeControlError::NodeNotRunning {
        name: canonical_node_name(index),
    })
}

#[async_trait::async_trait]
impl<E> NodeControlHandle<E> for K8sNodeControl<E>
where
    E: K8sDeployEnv,
{
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.restart(name).await.map_err(Into::into)
    }

    async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        ensure_default_start_options(&options)?;
        self.restart(name).await.map_err(Into::into)
    }

    async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.start(name).await.map_err(Into::into)
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        ensure_default_start_options(&options)?;
        self.start(name).await.map_err(Into::into)
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.stop(name).await.map_err(Into::into)
    }

    async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        let index = self.require_node_index(name)?;
        let _operation = self.operations[index].lock().await;
        {
            let running = self.running.lock().await;
            ensure_node_running(&running, index)?;
        }
        self.wait_node_ready_by_index(index)
            .await
            .map_err(Into::into)
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        let index = self.require_node_index(name).ok()?;
        self.build_client(index).ok()
    }
}

/// Builds a managed node control handle when required by the scenario
/// capabilities, or `None` otherwise.
pub(crate) fn maybe_managed_node_control<E: K8sDeployEnv>(
    required: bool,
    client: &Client,
    namespace: &str,
    release: &str,
    node_host: &str,
    node_api_ports: &[u16],
    node_auxiliary_ports: &[u16],
    forwards: PortForwardRegistry,
) -> Option<Arc<dyn NodeControlHandle<E>>> {
    required.then(|| {
        Arc::new(K8sNodeControl::<E>::new(
            client.clone(),
            namespace.to_owned(),
            release.to_owned(),
            node_host.to_owned(),
            node_api_ports.to_vec(),
            node_auxiliary_ports.to_vec(),
            forwards,
        )) as Arc<dyn NodeControlHandle<E>>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_start_options_are_accepted() {
        let options = StartNodeOptions::<crate::manual::tests_dummy_env::DummyEnv>::default();
        assert!(ensure_default_start_options(&options).is_ok());
    }

    #[test]
    fn persist_and_config_overrides_are_rejected() {
        let persist = StartNodeOptions::<crate::manual::tests_dummy_env::DummyEnv>::default()
            .with_persist_dir(std::path::PathBuf::from("/tmp/demo"));
        let override_config =
            StartNodeOptions::<crate::manual::tests_dummy_env::DummyEnv>::default()
                .with_config_override("override".to_owned());
        assert!(matches!(
            ensure_default_start_options(&persist),
            Err(K8sNodeControlError::UnsupportedStartOptions { .. })
        ));
        assert!(matches!(
            ensure_default_start_options(&override_config),
            Err(K8sNodeControlError::UnsupportedStartOptions { .. })
        ));
    }

    #[test]
    fn args_and_start_timeout_overrides_are_rejected() {
        let mut with_args = StartNodeOptions::<crate::manual::tests_dummy_env::DummyEnv>::default();
        with_args.args.push("--flag".to_owned());
        let mut with_timeout =
            StartNodeOptions::<crate::manual::tests_dummy_env::DummyEnv>::default();
        with_timeout.runtime.start_timeout = Some(std::time::Duration::from_secs(5));
        assert!(matches!(
            ensure_default_start_options(&with_args),
            Err(K8sNodeControlError::UnsupportedStartOptions { .. })
        ));
        assert!(matches!(
            ensure_default_start_options(&with_timeout),
            Err(K8sNodeControlError::UnsupportedStartOptions { .. })
        ));
    }

    #[test]
    fn running_state_rejects_stopped_nodes() {
        let running = HashSet::from([0]);

        assert!(ensure_node_running(&running, 0).is_ok());
        assert!(matches!(
            ensure_node_running(&running, 1),
            Err(K8sNodeControlError::NodeNotRunning { name }) if name == "node-1"
        ));
    }
}
