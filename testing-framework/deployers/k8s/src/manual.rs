use std::{
    collections::HashSet,
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use cfgsync_core::Client as CfgsyncClient;
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    Api, Client, Config,
    api::{Patch, PatchParams},
};
use reqwest::Url;
use testing_framework_core::{
    manual::ManualClusterHandle,
    naming::is_valid_cluster_name,
    scenario::{
        CleanupGuard, ClusterStartMode, ClusterWaitHandle, DeploymentPolicy, DynError,
        ExistingCluster, ExternalNodeSource, HttpReadinessRequirement, NodeAccess, NodeClients,
        NodeControl, NodeControlHandle, NodeLaunchOptions, ObservabilityInputs, PeerSelection,
        StartNodeOptions, StartedNode, StartedNodeAccess,
    },
};
use thiserror::Error;
use tokio_retry::{RetryIf, strategy::FixedInterval};
use tracing::warn;

use crate::{
    env::{
        K8sDeployEnv, attach_node_service_selector, build_cfgsync_override_artifacts,
        cfgsync_hostnames, cfgsync_service, cluster_identifiers, collect_port_specs,
        discovered_node_access, node_deployment_name, node_readiness_path, node_service_name,
        prepare_stack, wait_remote_readiness,
    },
    lifecycle::{
        cleanup::{CLEANUP_TIMEOUT, RunnerCleanup},
        logs::dump_namespace_logs,
        wait::{
            ClusterWaitError, NodeConfigPorts, NodePortAllocation, PortForwardRegistry,
            deployment::wait_for_deployment_ready, port_forward_service, wait_for_cluster_ready,
        },
    },
};

const LOCALHOST: &str = "127.0.0.1";

#[derive(Debug, Error)]
pub enum ManualClusterError {
    #[error("kubernetes runner requires at least one node (nodes={nodes})")]
    UnsupportedTopology { nodes: usize },
    #[error(
        "invalid k8s cluster name '{name}'; use a short lowercase DNS label (letters, digits, and dashes)"
    )]
    InvalidClusterName { name: String },
    #[error("failed to initialise kubernetes client: {source}")]
    ClientInit {
        #[source]
        source: kube::Error,
    },
    #[error("failed to prepare k8s assets: {source}")]
    Assets {
        #[source]
        source: DynError,
    },
    #[error("failed to install k8s stack: {source}")]
    InstallStack {
        #[source]
        source: DynError,
    },
    #[error("failed to update cfgsync artifacts for '{name}': {source}")]
    CfgsyncUpdate {
        name: String,
        #[source]
        source: DynError,
    },
    #[error(transparent)]
    NodePorts(#[from] ClusterWaitError),
    #[error("unsupported start options for k8s manual cluster: {message}")]
    UnsupportedStartOptions { message: String },
    #[error("invalid node name '{name}'; expected node-<index>")]
    InvalidNodeName { name: String },
    #[error("node index {index} is out of range for topology with {nodes} nodes")]
    NodeIndexOutOfRange { index: usize, nodes: usize },
    #[error("node '{name}' is already running")]
    NodeAlreadyRunning { name: String },
    #[error("node '{name}' is not running")]
    NodeNotRunning { name: String },
    #[error("failed to patch deployment {name}: {source}")]
    PatchDeployment {
        name: String,
        #[source]
        source: kube::Error,
    },
    #[error("node '{name}' did not reach {replicas} replicas before the scale timeout")]
    ReplicaScaleTimeout { name: String, replicas: i32 },
    #[error("failed to delete pods for deployment {name}: {source}")]
    DeletePods {
        name: String,
        #[source]
        source: kube::Error,
    },
    #[error("failed to discover node client for '{name}': {source}")]
    NodeClient {
        name: String,
        #[source]
        source: DynError,
    },
    #[error("node readiness failed for '{name}': {source}")]
    NodeReadiness {
        name: String,
        #[source]
        source: DynError,
    },
    #[error("cluster network readiness failed: {source}")]
    NetworkReadiness {
        #[source]
        source: DynError,
    },
    #[error("k8s manual cluster is no longer owned by an active run")]
    Closed,
}

struct ManualClusterState<E: K8sDeployEnv> {
    running: HashSet<usize>,
    node_clients: NodeClients<E>,
    known_clients: Vec<Option<E::NodeClient>>,
    inventory_slots: Vec<Option<usize>>,
    node_allocations: Vec<Option<NodePortAllocation>>,
}

pub struct ManualCluster<E: K8sDeployEnv> {
    client: Client,
    config: Config,
    namespace: String,
    release: String,
    topology: E::Deployment,
    node_count: usize,
    node_host: String,
    node_ports: Vec<NodeConfigPorts>,
    forwards: PortForwardRegistry,
    cleanup: Mutex<Option<RunnerCleanup>>,
    closed: AtomicBool,
    state: Arc<Mutex<ManualClusterState<E>>>,
}

struct ManualClusterCleanup<E: K8sDeployEnv> {
    cluster: Arc<ManualCluster<E>>,
}

impl<E: K8sDeployEnv> CleanupGuard for ManualClusterCleanup<E> {
    fn cleanup(self: Box<Self>) {
        self.cluster.close();
    }
}

struct EagerClusterParts<E: K8sDeployEnv> {
    node_host: String,
    node_allocations: Vec<NodePortAllocation>,
    port_forwards: PortForwardRegistry,
    node_clients: NodeClients<E>,
    known_clients: Vec<Option<E::NodeClient>>,
}

impl<E: K8sDeployEnv> ManualCluster<E> {
    pub async fn from_topology(topology: E::Deployment) -> Result<Self, ManualClusterError> {
        Self::provision(
            topology,
            ClusterStartMode::OnDemand,
            DeploymentPolicy::default(),
            &ObservabilityInputs::default(),
        )
        .await
    }

    pub async fn provision(
        topology: E::Deployment,
        start_mode: ClusterStartMode,
        policy: DeploymentPolicy,
        observability: &ObservabilityInputs,
    ) -> Result<Self, ManualClusterError> {
        Self::provision_named(topology, None, start_mode, policy, observability).await
    }

    pub(crate) async fn provision_named(
        topology: E::Deployment,
        cluster_name: Option<&str>,
        start_mode: ClusterStartMode,
        policy: DeploymentPolicy,
        observability: &ObservabilityInputs,
    ) -> Result<Self, ManualClusterError> {
        let nodes = testing_framework_core::topology::DeploymentDescriptor::node_count(&topology);
        if nodes == 0 {
            return Err(ManualClusterError::UnsupportedTopology { nodes });
        }
        if let Some(name) = cluster_name
            && !is_valid_cluster_name(name)
        {
            return Err(ManualClusterError::InvalidClusterName {
                name: name.to_owned(),
            });
        }

        crate::ensure_rustls_provider_installed();
        let config = Config::infer()
            .await
            .map_err(|source| ManualClusterError::ClientInit {
                source: kube::Error::InferConfig(source),
            })?;
        let client = Client::try_from(config.clone())
            .map_err(|source| ManualClusterError::ClientInit { source })?;
        let assets = prepare_stack::<E>(&topology, observability.metrics_otlp_ingest_url.as_ref())
            .map_err(|source| ManualClusterError::Assets { source })?;
        let (namespace, release) = cluster_identifiers::<E>(cluster_name);
        let cleanup = assets
            .install(&client, &namespace, &release, nodes)
            .await
            .map_err(|source| ManualClusterError::InstallStack { source })?;

        let node_ports = collect_port_specs::<E>(&topology).nodes;

        match start_mode {
            ClusterStartMode::OnDemand => {
                let scaled = scale_all_nodes::<E>(&client, &namespace, &release, nodes, 0).await;
                let ((), cleanup) = cleanup_if_failed(&client, &namespace, scaled, cleanup).await?;

                Ok(Self {
                    client,
                    config,
                    namespace,
                    release,
                    topology,
                    node_count: nodes,
                    node_host: LOCALHOST.to_owned(),
                    node_ports,
                    forwards: PortForwardRegistry::default(),
                    cleanup: Mutex::new(Some(cleanup)),
                    closed: AtomicBool::new(false),
                    state: Arc::new(Mutex::new(ManualClusterState {
                        running: HashSet::new(),
                        node_clients: NodeClients::default(),
                        known_clients: vec![None; nodes],
                        inventory_slots: vec![None; nodes],
                        node_allocations: vec![None; nodes],
                    })),
                })
            }
            ClusterStartMode::Eager => {
                let provisioned = provision_eager_parts::<E>(
                    &client,
                    &namespace,
                    &release,
                    &topology,
                    policy,
                    &node_ports,
                )
                .await;
                let (parts, cleanup) =
                    cleanup_if_failed(&client, &namespace, provisioned, cleanup).await?;

                Ok(Self {
                    client,
                    config,
                    namespace,
                    release,
                    topology,
                    node_count: nodes,
                    node_host: parts.node_host,
                    node_ports,
                    forwards: parts.port_forwards,
                    cleanup: Mutex::new(Some(cleanup)),
                    closed: AtomicBool::new(false),
                    state: Arc::new(Mutex::new(ManualClusterState {
                        running: (0..nodes).collect(),
                        node_clients: parts.node_clients,
                        known_clients: parts.known_clients,
                        inventory_slots: (0..nodes).map(Some).collect(),
                        node_allocations: parts.node_allocations.into_iter().map(Some).collect(),
                    })),
                })
            }
        }
    }

    #[must_use]
    pub fn cleanup_guard(self: &Arc<Self>) -> Box<dyn CleanupGuard> {
        Box::new(ManualClusterCleanup {
            cluster: Arc::clone(self),
        })
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        let index = parse_node_index(name)?;
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .known_clients
            .get(index)
            .and_then(|client| client.clone())
    }

    #[must_use]
    pub fn node_pid(&self, _name: &str) -> Option<u32> {
        None
    }

    pub async fn start_node(&self, name: &str) -> Result<StartedNode<E>, ManualClusterError> {
        self.start_node_with(name, StartNodeOptions::<E>::default())
            .await
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, ManualClusterError> {
        self.ensure_open()?;
        validate_start_options(&options)?;
        let index = self.require_node_index(name)?;
        {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.running.contains(&index) {
                return Err(ManualClusterError::NodeAlreadyRunning {
                    name: name.to_owned(),
                });
            }
        }

        self.apply_cfgsync_override(index, &options).await?;
        scale_node::<E>(&self.client, &self.namespace, &self.release, index, 1).await?;
        self.refresh_forwards(index).await?;
        self.wait_node_ready(name).await?;
        let client = self.build_client(index, name)?;

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.running.insert(index);
        record_node_client(&mut state, index, client.clone());

        Ok(StartedNode {
            name: canonical_node_name(index),
            client,
        })
    }

    pub fn stop_all(&self) {
        self.stop_all_blocking();
    }

    /// Runs the sequential per-node scale-down under one overall deadline so a
    /// degraded API server cannot stall close() far beyond the cleanup budget.
    ///
    /// Reuses `RunnerCleanup`'s timeout; when the deadline cuts teardown short
    /// a warning is logged and the caller proceeds with release and namespace
    /// cleanup.
    async fn stop_all_bounded(&self, client: &Client) {
        let result = tokio::time::timeout(CLEANUP_TIMEOUT, self.stop_all_with_client(client)).await;
        if result.is_err() {
            warn!(
                timeout_secs = CLEANUP_TIMEOUT.as_secs(),
                "node scale-down did not finish before the teardown deadline; proceeding with cleanup"
            );
        }
    }

    async fn stop_all_with_client(&self, client: &Client) -> Result<(), ManualClusterError> {
        let indices = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.running.iter().copied().collect::<Vec<_>>()
        };

        for index in indices {
            let name = canonical_node_name(index);
            self.stop_node_with_client(client, &name).await?;
        }

        Ok(())
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), ManualClusterError> {
        self.restart_node_with(name, StartNodeOptions::<E>::default())
            .await
    }

    /// Restarts a running node with the given start options.
    ///
    /// Options the k8s backend can honor (peer selection, config overrides,
    /// config patches) are applied through cfgsync before the node's pod is
    /// replaced; options it cannot honor (persist/snapshot directories, extra
    /// process arguments, start timeout overrides) are rejected, mirroring the
    /// legacy managed k8s node control.
    pub async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), ManualClusterError> {
        self.ensure_open()?;
        validate_restart_options(&options)?;
        let index = self.require_running_node_index(name)?;
        self.apply_cfgsync_override(index, &options).await?;
        scale_node::<E>(&self.client, &self.namespace, &self.release, index, 0).await?;
        scale_node::<E>(&self.client, &self.namespace, &self.release, index, 1).await?;
        self.refresh_forwards(index).await?;
        self.wait_node_ready(name).await?;
        let client = self.build_client(index, name)?;

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        record_node_client(&mut state, index, client);
        Ok(())
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), ManualClusterError> {
        self.ensure_open()?;
        self.stop_node_inner(name).await
    }

    async fn stop_node_inner(&self, name: &str) -> Result<(), ManualClusterError> {
        self.stop_node_with_client(&self.client, name).await
    }

    async fn stop_node_with_client(
        &self,
        client: &Client,
        name: &str,
    ) -> Result<(), ManualClusterError> {
        let index = self.require_running_node_index(name)?;
        scale_node::<E>(client, &self.namespace, &self.release, index, 0).await?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.running.remove(&index);
        Ok(())
    }

    async fn refresh_forwards(&self, index: usize) -> Result<(), ManualClusterError> {
        if self.node_host != LOCALHOST {
            return Ok(());
        }

        let forwards = self.forwards.clone();
        let namespace = self.namespace.clone();
        let service = node_service_name::<E>(&self.release, index);
        let ports = self.node_ports[index];
        let allocation = tokio::task::spawn_blocking(move || {
            forwards.forward_node(index, &namespace, &service, ports)
        })
        .await
        .map_err(|source| {
            ManualClusterError::NodePorts(ClusterWaitError::PortForwardTask {
                source: source.into(),
            })
        })??;

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.node_allocations[index] = Some(allocation);
        Ok(())
    }

    fn ensure_open(&self) -> Result<(), ManualClusterError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ManualClusterError::Closed);
        }
        Ok(())
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stop_all_blocking();
        self.forwards.shutdown_all();
        if let Some(cleanup) = take_cleanup(&self.cleanup) {
            CleanupGuard::cleanup(Box::new(cleanup));
        }
    }

    /// Drives a best-effort node scale-down to completion from synchronous
    /// code.
    ///
    /// `block_in_place` panics on current-thread tokio runtimes, which would
    /// abort cleanup mid-run, so that flavor runs the scale-down on a
    /// dedicated thread with its own small runtime instead. Code outside any
    /// runtime blocks on a fresh current-thread runtime with the pooled
    /// client: its connections are either still driven by the (unblocked)
    /// owning runtime or fail fast once that runtime is gone.
    fn stop_all_blocking(&self) {
        match tokio::runtime::Handle::try_current() {
            Ok(handle)
                if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::CurrentThread =>
            {
                tokio::task::block_in_place(|| {
                    handle.block_on(self.stop_all_bounded(&self.client));
                });
            }
            Ok(_) => self.stop_all_on_dedicated_thread(),
            Err(_) => {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    runtime.block_on(self.stop_all_bounded(&self.client));
                }
            }
        }
    }

    /// Scales nodes down from a dedicated thread while the calling
    /// current-thread runtime stays blocked.
    ///
    /// The pooled client must not be reused here: its connections are driven
    /// by tasks on the blocked outer runtime, so requests through it could
    /// stall forever. A fresh client built from the stored config keeps every
    /// connection on the dedicated runtime; if building it fails the
    /// scale-down is skipped with a warning instead of hanging.
    fn stop_all_on_dedicated_thread(&self) {
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(async {
                    match Client::try_from(self.config.clone()) {
                        Ok(client) => self.stop_all_bounded(&client).await,
                        Err(error) => warn!(
                            error = ?error,
                            "failed to build a dedicated cleanup client; skipping node scale-down"
                        ),
                    }
                });
            });
        });
    }

    pub async fn wait_network_ready(&self) -> Result<(), ManualClusterError> {
        self.ensure_open()?;
        let (running_ports, registered_nodes) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let ports = state
                .running
                .iter()
                .copied()
                .map(|index| {
                    state.node_allocations[index]
                        .map(|allocation| allocation.api)
                        .ok_or_else(|| ManualClusterError::NodeClient {
                            name: canonical_node_name(index),
                            source: "node has no active port-forward allocation".into(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let registered = state.inventory_slots.iter().flatten().count();
            (ports, registered)
        };

        if running_ports.is_empty() {
            if registered_nodes == 0 {
                return Ok(());
            }
            return Err(ManualClusterError::NetworkReadiness {
                source: format!(
                    "all {} nodes are stopped; no running nodes to await readiness",
                    self.node_count
                )
                .into(),
            });
        }

        let ports = running_ports;
        testing_framework_core::scenario::wait_for_http_ports_with_host_and_requirement(
            &ports,
            &self.node_host,
            node_readiness_path::<E>(),
            HttpReadinessRequirement::AllNodesReady,
        )
        .await
        .map_err(|source| ManualClusterError::NetworkReadiness {
            source: source.into(),
        })
    }

    pub async fn wait_node_ready(&self, name: &str) -> Result<(), ManualClusterError> {
        self.ensure_open()?;
        let index = self.require_node_index(name)?;
        let port = self.node_allocation(index)?.api;
        testing_framework_core::scenario::wait_for_http_ports_with_host_and_requirement(
            &[port],
            &self.node_host,
            node_readiness_path::<E>(),
            HttpReadinessRequirement::AllNodesReady,
        )
        .await
        .map_err(|source| ManualClusterError::NodeReadiness {
            name: canonical_node_name(index),
            source: source.into(),
        })
    }

    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.node_clients.clone()
    }

    /// Returns the descriptor a later attached cluster request can consume to
    /// re-attach to this deployment's node services.
    #[must_use]
    pub fn attachment(&self) -> ExistingCluster {
        ExistingCluster::for_k8s_selector_in_namespace(
            self.namespace.clone(),
            attach_node_service_selector::<E>(&self.release),
        )
    }

    pub fn add_external_sources(
        &self,
        external_sources: impl IntoIterator<Item = ExternalNodeSource>,
    ) -> Result<(), DynError> {
        let clients = external_sources
            .into_iter()
            .map(|source| E::external_node_client(&source))
            .collect::<Result<Vec<_>, _>>()?;
        self.add_external_clients(clients);
        Ok(())
    }

    /// Appends external clients while holding the state lock so the appends
    /// cannot interleave with `record_node_client`'s slot bookkeeping.
    pub fn add_external_clients(&self, clients: impl IntoIterator<Item = E::NodeClient>) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for client in clients {
            state.node_clients.add_node(client);
        }
    }

    fn build_client(&self, index: usize, name: &str) -> Result<E::NodeClient, ManualClusterError> {
        let allocation = self.node_allocation(index)?;
        E::build_node_client(&discovered_node_access(
            &self.node_host,
            allocation.api,
            allocation.auxiliary,
        ))
        .map_err(|source| ManualClusterError::NodeClient {
            name: name.to_owned(),
            source,
        })
    }

    fn node_allocation(&self, index: usize) -> Result<NodePortAllocation, ManualClusterError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.node_allocations[index].ok_or_else(|| ManualClusterError::NodeClient {
            name: canonical_node_name(index),
            source: "node has no active port-forward allocation".into(),
        })
    }

    fn require_node_index(&self, name: &str) -> Result<usize, ManualClusterError> {
        let index = parse_node_index(name).ok_or_else(|| ManualClusterError::InvalidNodeName {
            name: name.to_owned(),
        })?;
        if index >= self.node_count {
            return Err(ManualClusterError::NodeIndexOutOfRange {
                index,
                nodes: self.node_count,
            });
        }
        Ok(index)
    }

    fn require_running_node_index(&self, name: &str) -> Result<usize, ManualClusterError> {
        let index = self.require_node_index(name)?;
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.running.contains(&index) {
            return Err(ManualClusterError::NodeNotRunning {
                name: canonical_node_name(index),
            });
        }
        Ok(index)
    }

    async fn apply_cfgsync_override(
        &self,
        index: usize,
        options: &StartNodeOptions<E>,
    ) -> Result<(), ManualClusterError> {
        let Some((service, port)) = cfgsync_service::<E>(&self.release) else {
            return ensure_default_cfgsync_options(options);
        };

        let hostnames = cfgsync_hostnames::<E>(&self.release, self.node_count);
        let artifacts =
            build_cfgsync_override_artifacts::<E>(&self.topology, index, &hostnames, options)
                .map_err(|source| ManualClusterError::CfgsyncUpdate {
                    name: canonical_node_name(index),
                    source,
                })?;

        let Some(artifacts) = artifacts else {
            return ensure_default_cfgsync_options(options);
        };

        let forward = port_forward_service(&self.namespace, &service, port)?;
        let client = CfgsyncClient::new(format!(
            "http://{}:{}",
            Ipv4Addr::LOCALHOST,
            forward.local_port
        ));

        client
            .replace_node_artifacts(canonical_node_name(index), artifacts.files)
            .await
            .map_err(|source| ManualClusterError::CfgsyncUpdate {
                name: canonical_node_name(index),
                source: source.into(),
            })?;

        Ok(())
    }
}

impl<E> Drop for ManualCluster<E>
where
    E: K8sDeployEnv,
{
    fn drop(&mut self) {
        self.close();
    }
}

#[async_trait::async_trait]
impl<E> NodeControl for ManualCluster<E>
where
    E: K8sDeployEnv,
{
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        Self::restart_node(self, name).await.map_err(Into::into)
    }

    async fn restart_node_with(
        &self,
        name: &str,
        options: NodeLaunchOptions,
    ) -> Result<(), DynError> {
        self.restart_node_with_config(name, options.into()).await
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: NodeLaunchOptions,
    ) -> Result<StartedNodeAccess, DynError> {
        let started = self.start_node_with_config(name, options.into()).await?;
        let access = NodeControl::node_access(self, &started.name).await?;
        Ok(StartedNodeAccess {
            name: started.name,
            access,
        })
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        Self::stop_node(self, name).await.map_err(Into::into)
    }

    async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        self.ensure_open()?;
        self.require_running_node_index(name)?;
        Self::wait_node_ready(self, name).await.map_err(Into::into)
    }

    async fn node_access(&self, name: &str) -> Result<NodeAccess, DynError> {
        self.ensure_open()?;
        let index = self.require_node_index(name)?;
        let allocation = self.node_allocation(index)?;
        Ok(discovered_node_access(
            &self.node_host,
            allocation.api,
            allocation.auxiliary,
        ))
    }

    fn node_names(&self) -> Vec<String> {
        (0..self.node_count).map(canonical_node_name).collect()
    }
}

#[async_trait::async_trait]
impl<E> NodeControlHandle<E> for ManualCluster<E>
where
    E: K8sDeployEnv,
{
    async fn restart_node_with_config(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        Self::restart_node_with(self, name, options)
            .await
            .map_err(Into::into)
    }

    async fn start_node_with_config(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        Self::start_node_with(self, name, options)
            .await
            .map_err(Into::into)
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        Self::node_client(self, name)
    }
}

#[async_trait::async_trait]
impl<E> ClusterWaitHandle for ManualCluster<E>
where
    E: K8sDeployEnv,
{
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        Self::wait_network_ready(self).await.map_err(Into::into)
    }
}

#[async_trait::async_trait]
impl<E> ManualClusterHandle<E> for ManualCluster<E> where E: K8sDeployEnv {}

fn take_cleanup(cleanup: &Mutex<Option<RunnerCleanup>>) -> Option<RunnerCleanup> {
    cleanup
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

async fn run_failed_provision_cleanup<C: CleanupGuard + Send + 'static>(cleanup: C) {
    let _ = tokio::task::spawn_blocking(move || {
        CleanupGuard::cleanup(Box::new(cleanup));
    })
    .await;
}

/// Passes through a successful provisioning step, or dumps namespace pod
/// logs and runs the installed stack's cleanup before propagating the error
/// so failed provisioning keeps its diagnostics and does not leak the Helm
/// release and namespace.
async fn cleanup_if_failed<T, C: CleanupGuard + Send + 'static>(
    client: &Client,
    namespace: &str,
    result: Result<T, ManualClusterError>,
    cleanup: C,
) -> Result<(T, C), ManualClusterError> {
    match result {
        Ok(value) => Ok((value, cleanup)),
        Err(error) => {
            dump_namespace_logs(client, namespace).await;
            run_failed_provision_cleanup(cleanup).await;
            Err(error)
        }
    }
}

async fn provision_eager_parts<E: K8sDeployEnv>(
    client: &Client,
    namespace: &str,
    release: &str,
    topology: &E::Deployment,
    policy: DeploymentPolicy,
    node_ports: &[NodeConfigPorts],
) -> Result<EagerClusterParts<E>, ManualClusterError> {
    let ready = wait_for_cluster_ready::<E>(client, namespace, release, node_ports).await?;
    let node_host = ready.ports.node_host;
    let node_allocations = ready.ports.nodes;
    let port_forwards = ready.port_forwards;

    if policy.readiness_enabled {
        let api_ports = node_allocations
            .iter()
            .map(|allocation| allocation.api)
            .collect::<Vec<_>>();
        if let Err(error) = wait_policy_readiness::<E>(
            topology,
            &node_host,
            &api_ports,
            policy.readiness_requirement,
        )
        .await
        {
            port_forwards.shutdown_all_async().await;
            return Err(error);
        }
    }

    let node_clients = NodeClients::default();
    let mut known_clients = Vec::with_capacity(node_allocations.len());
    for (index, allocation) in node_allocations.iter().enumerate() {
        let access = discovered_node_access(&node_host, allocation.api, allocation.auxiliary);
        match E::build_node_client(&access) {
            Ok(node_client) => {
                known_clients.push(Some(node_client.clone()));
                node_clients.add_node(node_client);
            }
            Err(source) => {
                port_forwards.shutdown_all_async().await;
                return Err(ManualClusterError::NodeClient {
                    name: canonical_node_name(index),
                    source,
                });
            }
        }
    }

    Ok(EagerClusterParts {
        node_host,
        node_allocations,
        port_forwards,
        node_clients,
        known_clients,
    })
}

async fn wait_policy_readiness<E: K8sDeployEnv>(
    topology: &E::Deployment,
    node_host: &str,
    api_ports: &[u16],
    requirement: HttpReadinessRequirement,
) -> Result<(), ManualClusterError> {
    let urls = api_ports
        .iter()
        .map(|port| Url::parse(&format!("http://{node_host}:{port}/")))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ManualClusterError::NetworkReadiness {
            source: source.into(),
        })?;

    wait_remote_readiness::<E>(topology, &urls, requirement)
        .await
        .map_err(|source| ManualClusterError::NetworkReadiness { source })
}

async fn scale_all_nodes<E: K8sDeployEnv>(
    client: &Client,
    namespace: &str,
    release: &str,
    node_count: usize,
    replicas: i32,
) -> Result<(), ManualClusterError> {
    for index in 0..node_count {
        scale_node::<E>(client, namespace, release, index, replicas).await?;
    }
    Ok(())
}

/// Patches the per-node deployment to the requested replica count and waits
/// for the deployment to reach it.
pub(crate) async fn scale_node<E: K8sDeployEnv>(
    client: &Client,
    namespace: &str,
    release: &str,
    index: usize,
    replicas: i32,
) -> Result<(), ManualClusterError> {
    let name = node_deployment_name::<E>(release, index);
    scale_deployment(
        client,
        namespace,
        &name,
        &canonical_node_name(index),
        replicas,
    )
    .await
}

/// Patches the named deployment to the requested replica count and waits for
/// the deployment to reach it.
pub(crate) async fn scale_deployment(
    client: &Client,
    namespace: &str,
    deployment_name: &str,
    node_name: &str,
    replicas: i32,
) -> Result<(), ManualClusterError> {
    patch_node_replicas(client, namespace, deployment_name, replicas).await?;
    wait_for_replicas(client, namespace, deployment_name, node_name, replicas).await
}

pub(crate) async fn patch_node_replicas(
    client: &Client,
    namespace: &str,
    deployment_name: &str,
    replicas: i32,
) -> Result<(), ManualClusterError> {
    let deployments = Api::<Deployment>::namespaced(client.clone(), namespace);
    let patch = serde_json::json!({"spec": {"replicas": replicas}});
    deployments
        .patch(
            deployment_name,
            &PatchParams::default(),
            &Patch::Merge(&patch),
        )
        .await
        .map_err(|source| ManualClusterError::PatchDeployment {
            name: deployment_name.to_owned(),
            source,
        })?;
    Ok(())
}

pub(crate) async fn wait_for_replicas(
    client: &Client,
    namespace: &str,
    deployment_name: &str,
    node_name: &str,
    replicas: i32,
) -> Result<(), ManualClusterError> {
    if replicas > 0 {
        return wait_for_deployment_ready(client, namespace, deployment_name)
            .await
            .map_err(Into::into);
    }

    let deployments = Api::<Deployment>::namespaced(client.clone(), namespace);
    let result = RetryIf::start(
        FixedInterval::from_millis(500).take(240),
        || async {
            let deployment = deployments.get(deployment_name).await.map_err(|source| {
                ManualClusterError::PatchDeployment {
                    name: deployment_name.to_owned(),
                    source,
                }
            })?;
            let ready = deployment
                .status
                .as_ref()
                .and_then(|status| status.ready_replicas)
                .unwrap_or(0);
            let current = deployment
                .spec
                .as_ref()
                .and_then(|spec| spec.replicas)
                .unwrap_or(1);
            if ready == 0 && current == 0 {
                Ok(())
            } else {
                Err(ManualClusterError::NodeAlreadyRunning {
                    name: node_name.to_owned(),
                })
            }
        },
        |error: &ManualClusterError| matches!(error, ManualClusterError::NodeAlreadyRunning { .. }),
    )
    .await;

    match result {
        Err(ManualClusterError::NodeAlreadyRunning { .. }) => {
            Err(ManualClusterError::ReplicaScaleTimeout {
                name: node_name.to_owned(),
                replicas,
            })
        }
        other => other,
    }
}

fn validate_start_options<E: K8sDeployEnv>(
    options: &StartNodeOptions<E>,
) -> Result<(), ManualClusterError> {
    if options.common.persist_dir.is_some() || options.common.snapshot_dir.is_some() {
        return Err(ManualClusterError::UnsupportedStartOptions {
            message: "persist/snapshot directories are not supported".to_owned(),
        });
    }
    Ok(())
}

/// Rejects restart options the k8s backend cannot honor: a restarted pod is
/// relaunched with the container arguments and timeouts baked into its
/// deployment, so accepting them would report a configured restart that never
/// happened.
pub(crate) fn validate_restart_options<E: K8sDeployEnv>(
    options: &StartNodeOptions<E>,
) -> Result<(), ManualClusterError> {
    validate_start_options(options)?;
    if !options.common.args.is_empty() {
        return Err(ManualClusterError::UnsupportedStartOptions {
            message: "extra process arguments are not supported on restart".to_owned(),
        });
    }
    if options.common.runtime.start_timeout.is_some() {
        return Err(ManualClusterError::UnsupportedStartOptions {
            message: "start timeout overrides are not supported on restart".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn ensure_default_cfgsync_options<E: K8sDeployEnv>(
    options: &StartNodeOptions<E>,
) -> Result<(), ManualClusterError> {
    let default_peers = matches!(
        options.common.peers,
        None | Some(PeerSelection::DefaultLayout)
    );
    if default_peers && options.config_override.is_none() && options.config_patch.is_none() {
        return Ok(());
    }

    Err(ManualClusterError::UnsupportedStartOptions {
        message: "cfgsync override support is not configured for these start options".to_owned(),
    })
}

/// Records a node's client after a start or restart, replacing the node's
/// existing entry in the shared inventory instead of appending a duplicate.
fn record_node_client<E: K8sDeployEnv>(
    state: &mut ManualClusterState<E>,
    index: usize,
    client: E::NodeClient,
) {
    state.known_clients[index] = Some(client.clone());
    if let Some(slot) = state.inventory_slots[index]
        && state.node_clients.replace_node(slot, client.clone())
    {
        return;
    }
    state.inventory_slots[index] = Some(state.node_clients.len());
    state.node_clients.add_node(client);
}

/// Parses a canonical `node-<index>` name into its index.
pub(crate) fn parse_node_index(name: &str) -> Option<usize> {
    name.strip_prefix("node-")?.parse().ok()
}

/// Formats the canonical `node-<index>` name for a node index.
pub(crate) fn canonical_node_name(index: usize) -> String {
    format!("node-{index}")
}

/// Minimal `K8sDeployEnv` implementation shared by unit tests in this crate.
#[cfg(test)]
pub(crate) mod tests_dummy_env {
    use testing_framework_core::{
        cfgsync::{StaticNodeConfigProvider, build_node_artifact_override},
        scenario::{Application, DynError, NodeAccess, PeerSelection, StartNodeOptions},
    };

    use crate::{
        RenderedHelmChartAssets, env::K8sDeployEnv, render_single_template_chart_assets,
        standard_port_specs,
    };

    pub(crate) struct DummyEnv;

    #[async_trait::async_trait]
    impl Application for DummyEnv {
        type Deployment = testing_framework_core::topology::ClusterTopology;
        type NodeClient = String;
        type NodeConfig = String;

        fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
            Ok(access.api_base_url()?.to_string())
        }
    }

    #[async_trait::async_trait]
    impl K8sDeployEnv for DummyEnv {
        type Assets = RenderedHelmChartAssets;

        fn collect_port_specs(
            _topology: &Self::Deployment,
        ) -> crate::infrastructure::cluster::PortSpecs {
            standard_port_specs(1, 8080, 8081)
        }

        fn prepare_assets(
            _topology: &Self::Deployment,
            _metrics_otlp_ingest_url: Option<&reqwest::Url>,
        ) -> Result<Self::Assets, DynError> {
            render_single_template_chart_assets("dummy", "dummy.yaml", "")
        }

        fn cfgsync_service(release: &str) -> Option<(String, u16)> {
            Some((format!("{release}-cfgsync"), 4400))
        }

        fn build_cfgsync_override_artifacts(
            topology: &Self::Deployment,
            node_index: usize,
            hostnames: &[String],
            options: &testing_framework_core::scenario::StartNodeOptions<Self>,
        ) -> Result<Option<cfgsync_artifacts::ArtifactSet>, DynError> {
            build_node_artifact_override::<Self>(topology, node_index, hostnames, options)
                .map_err(Into::into)
        }
    }

    impl StaticNodeConfigProvider for DummyEnv {
        type Error = std::io::Error;

        fn build_node_config(
            _deployment: &Self::Deployment,
            node_index: usize,
        ) -> Result<Self::NodeConfig, Self::Error> {
            Ok(format!("node={node_index};peers=default"))
        }

        fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error> {
            Ok(config.clone())
        }

        fn build_node_artifacts_for_options(
            _deployment: &Self::Deployment,
            node_index: usize,
            _hostnames: &[String],
            options: &StartNodeOptions<Self>,
        ) -> Result<Option<cfgsync_artifacts::ArtifactSet>, Self::Error> {
            let mut config = match &options.common.peers {
                None | Some(PeerSelection::DefaultLayout) => {
                    if options.config_override.is_none() && options.config_patch.is_none() {
                        return Ok(None);
                    }
                    format!("node={node_index};peers=default")
                }
                Some(PeerSelection::None) => format!("node={node_index};peers=none"),
                Some(PeerSelection::Named(names)) => {
                    format!("node={node_index};peers={}", names.join(","))
                }
            };
            if let Some(override_config) = options.config_override.clone() {
                config = override_config;
            }
            if let Some(config_patch) = &options.config_patch {
                config = config_patch(config).map_err(|source| {
                    std::io::Error::other(format!("failed to patch dummy config: {source}"))
                })?;
            }
            Ok(Some(cfgsync_artifacts::ArtifactSet::new(vec![
                cfgsync_artifacts::ArtifactFile::new("/config.yaml".to_string(), config),
            ])))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use testing_framework_core::scenario::PeerSelection;

    use super::{tests_dummy_env::DummyEnv, *};

    #[tokio::test]
    async fn invalid_cluster_name_is_rejected_before_kubernetes_access() {
        let result = ManualCluster::<DummyEnv>::provision_named(
            testing_framework_core::topology::ClusterTopology::new(1),
            Some("Bad_Name"),
            ClusterStartMode::Eager,
            DeploymentPolicy::default(),
            &ObservabilityInputs::default(),
        )
        .await;

        let Err(error) = result else {
            panic!("invalid cluster name must be rejected");
        };
        assert!(matches!(
            error,
            ManualClusterError::InvalidClusterName { name } if name == "Bad_Name"
        ));
    }

    #[test]
    fn parse_node_index_accepts_node_labels() {
        assert_eq!(parse_node_index("node-0"), Some(0));
        assert_eq!(parse_node_index("node-12"), Some(12));
        assert_eq!(parse_node_index("validator-0"), None);
    }

    #[test]
    fn validate_start_options_accepts_config_overrides() {
        let override_config =
            StartNodeOptions::<DummyEnv>::default().with_config_override("override".to_owned());
        let patched = StartNodeOptions::<DummyEnv>::default().create_patch(|mut config| {
            config.push_str(";patched");
            Ok(config)
        });

        assert!(validate_start_options(&override_config).is_ok());
        assert!(validate_start_options(&patched).is_ok());
    }

    #[test]
    fn validate_start_options_rejects_persist_and_snapshot_dirs() {
        let persist = StartNodeOptions::<DummyEnv>::default()
            .with_persist_dir(std::path::PathBuf::from("/tmp/demo"));
        let snapshot = StartNodeOptions::<DummyEnv>::default()
            .with_snapshot_dir(std::path::PathBuf::from("/tmp/snapshot"));
        assert!(matches!(
            validate_start_options(&persist),
            Err(ManualClusterError::UnsupportedStartOptions { .. })
        ));
        assert!(matches!(
            validate_start_options(&snapshot),
            Err(ManualClusterError::UnsupportedStartOptions { .. })
        ));
    }

    #[test]
    fn ensure_default_cfgsync_options_rejects_non_default_overrides() {
        let peers = StartNodeOptions::<DummyEnv>::default()
            .with_peers(PeerSelection::Named(vec!["node-0".to_owned()]));
        let override_config =
            StartNodeOptions::<DummyEnv>::default().with_config_override("override".to_owned());
        assert!(matches!(
            ensure_default_cfgsync_options(&peers),
            Err(ManualClusterError::UnsupportedStartOptions { .. })
        ));
        assert!(matches!(
            ensure_default_cfgsync_options(&override_config),
            Err(ManualClusterError::UnsupportedStartOptions { .. })
        ));
    }

    #[test]
    fn dummy_env_builds_cfgsync_override_artifacts() {
        let topology = testing_framework_core::topology::ClusterTopology::new(2);
        let options = StartNodeOptions::<DummyEnv>::default()
            .with_peers(PeerSelection::Named(vec!["node-0".to_owned()]));

        let artifacts = crate::env::build_cfgsync_override_artifacts::<DummyEnv>(
            &topology,
            1,
            &["node-0".to_owned(), "node-1".to_owned()],
            &options,
        )
        .expect("build override")
        .expect("expected override");

        assert_eq!(artifacts.files.len(), 1);
        assert_eq!(artifacts.files[0].content, "node=1;peers=node-0");
    }

    fn offline_cluster() -> ManualCluster<DummyEnv> {
        crate::ensure_rustls_provider_installed();
        let config = kube::Config::new("http://127.0.0.1:1".parse().expect("cluster url"));
        let client = Client::try_from(config.clone()).expect("offline kube client");
        let cleanup =
            RunnerCleanup::new(client.clone(), "ns".to_owned(), "release".to_owned(), true);
        ManualCluster {
            client,
            config,
            namespace: "ns".to_owned(),
            release: "release".to_owned(),
            topology: testing_framework_core::topology::ClusterTopology::new(1),
            node_count: 1,
            node_host: "127.0.0.1".to_owned(),
            node_ports: vec![NodeConfigPorts {
                api: 8080,
                auxiliary: 8081,
            }],
            forwards: PortForwardRegistry::default(),
            cleanup: Mutex::new(Some(cleanup)),
            closed: AtomicBool::new(false),
            state: Arc::new(Mutex::new(ManualClusterState {
                running: HashSet::new(),
                node_clients: NodeClients::default(),
                known_clients: vec![None],
                inventory_slots: vec![None],
                node_allocations: vec![Some(NodePortAllocation {
                    api: 1,
                    auxiliary: 2,
                })],
            })),
        }
    }

    #[tokio::test]
    async fn close_with_running_node_completes_on_current_thread_runtime() {
        let cluster = Arc::new(offline_cluster());
        {
            let mut state = cluster
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.running.insert(0);
        }
        let guard = cluster.cleanup_guard();

        guard.cleanup();

        assert!(take_cleanup(&cluster.cleanup).is_none());
        assert!(cluster.closed.load(Ordering::Acquire));
        drop(cluster);
    }

    #[tokio::test]
    async fn external_client_appends_keep_recorded_slots_stable() {
        let cluster = offline_cluster();

        cluster.add_external_clients(vec!["http://external-0/".to_owned()]);
        {
            let mut state = cluster
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            record_node_client(&mut state, 0, "http://managed-old/".to_owned());
        }
        cluster.add_external_clients(vec!["http://external-1/".to_owned()]);
        {
            let mut state = cluster
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            record_node_client(&mut state, 0, "http://managed-new/".to_owned());
        }

        let state = cluster
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            state.node_clients.snapshot(),
            vec![
                "http://external-0/".to_owned(),
                "http://managed-new/".to_owned(),
                "http://external-1/".to_owned(),
            ]
        );
        assert_eq!(state.inventory_slots[0], Some(1));
    }

    #[tokio::test]
    async fn cleanup_guard_does_not_panic_on_current_thread_runtime() {
        let cluster = Arc::new(offline_cluster());
        let guard = cluster.cleanup_guard();

        guard.cleanup();

        assert!(take_cleanup(&cluster.cleanup).is_none());
        assert!(cluster.closed.load(Ordering::Acquire));
        drop(cluster);
    }

    #[tokio::test]
    async fn cleanup_if_failed_runs_cleanup_only_on_error() {
        struct CountingCleanup(Arc<std::sync::atomic::AtomicUsize>);

        impl CleanupGuard for CountingCleanup {
            fn cleanup(self: Box<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        crate::ensure_rustls_provider_installed();
        let config = kube::Config::new("http://127.0.0.1:1".parse().expect("cluster url"));
        let client = Client::try_from(config).expect("offline kube client");

        let ok = cleanup_if_failed(&client, "ns", Ok(7), CountingCleanup(Arc::clone(&calls))).await;
        let (value, kept) = ok.expect("success must pass through");
        assert_eq!(value, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        CleanupGuard::cleanup(Box::new(kept));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let failed = cleanup_if_failed::<usize, _>(
            &client,
            "ns",
            Err(ManualClusterError::Closed),
            CountingCleanup(Arc::clone(&calls)),
        )
        .await;
        assert!(matches!(failed, Err(ManualClusterError::Closed)));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn record_node_client_replaces_entry_after_restart() {
        let mut state = ManualClusterState::<DummyEnv> {
            running: HashSet::from([0]),
            node_clients: NodeClients::new(vec!["http://old/".to_owned()]),
            known_clients: vec![Some("http://old/".to_owned())],
            inventory_slots: vec![Some(0)],
            node_allocations: vec![Some(NodePortAllocation {
                api: 1,
                auxiliary: 2,
            })],
        };

        record_node_client(&mut state, 0, "http://new/".to_owned());

        assert_eq!(state.node_clients.len(), 1);
        assert_eq!(
            state.node_clients.snapshot(),
            vec!["http://new/".to_owned()]
        );
        assert_eq!(state.known_clients[0].as_deref(), Some("http://new/"));
        assert_eq!(state.inventory_slots[0], Some(0));
    }

    #[test]
    fn record_node_client_appends_once_for_first_start() {
        let mut state = ManualClusterState::<DummyEnv> {
            running: HashSet::new(),
            node_clients: NodeClients::default(),
            known_clients: vec![None],
            inventory_slots: vec![None],
            node_allocations: vec![None],
        };

        record_node_client(&mut state, 0, "http://first/".to_owned());
        record_node_client(&mut state, 0, "http://second/".to_owned());

        assert_eq!(state.node_clients.len(), 1);
        assert_eq!(
            state.node_clients.snapshot(),
            vec!["http://second/".to_owned()]
        );
        assert_eq!(state.inventory_slots[0], Some(0));
    }

    #[test]
    fn validate_restart_options_rejects_args_and_timeout_overrides() {
        let with_args = StartNodeOptions::<DummyEnv>::default().with_args(["--flag".to_owned()]);
        let with_timeout = StartNodeOptions::<DummyEnv>::default()
            .with_start_timeout(std::time::Duration::from_secs(5));
        assert!(matches!(
            validate_restart_options(&with_args),
            Err(ManualClusterError::UnsupportedStartOptions { .. })
        ));
        assert!(matches!(
            validate_restart_options(&with_timeout),
            Err(ManualClusterError::UnsupportedStartOptions { .. })
        ));
        assert!(validate_restart_options(&StartNodeOptions::<DummyEnv>::default()).is_ok());
    }

    #[tokio::test]
    async fn node_control_restart_node_with_reaches_backend() {
        let cluster = offline_cluster();
        {
            let mut state = cluster
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.running.insert(0);
        }

        let error = NodeControlHandle::restart_node_with_config(
            &cluster,
            "node-0",
            StartNodeOptions::<DummyEnv>::default(),
        )
        .await
        .expect_err("offline restart must fail against the unreachable API server");

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "restart_node_with must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("failed to patch deployment"),
            "expected a real scale attempt error, got: {message}"
        );
    }

    #[tokio::test]
    async fn node_control_wait_node_ready_rejects_stopped_node() {
        let cluster = offline_cluster();

        let error = NodeControl::wait_node_ready(&cluster, "node-0")
            .await
            .expect_err("waiting on a stopped node must fail");

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "wait_node_ready must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("is not running"),
            "expected a stopped-node error, got: {message}"
        );
    }

    #[tokio::test]
    async fn node_control_wait_node_ready_requires_allocation() {
        let cluster = offline_cluster();
        {
            let mut state = cluster
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.running.insert(0);
            state.node_allocations[0] = None;
        }

        let error = NodeControl::wait_node_ready(&cluster, "node-0")
            .await
            .expect_err("waiting without a port-forward allocation must fail");

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "wait_node_ready must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("no active port-forward allocation"),
            "expected a missing-allocation error, got: {message}"
        );
    }

    #[tokio::test]
    async fn wait_network_ready_errors_when_all_nodes_stopped() {
        let cluster = offline_cluster();
        {
            let mut state = cluster
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.inventory_slots[0] = Some(0);
        }

        let error = cluster
            .wait_network_ready()
            .await
            .expect_err("an all-stopped cluster must not report readiness");

        assert!(matches!(error, ManualClusterError::NetworkReadiness { .. }));
        assert!(error.to_string().contains("all 1 nodes are stopped"));
    }

    #[tokio::test]
    async fn wait_network_ready_is_ok_before_any_node_started() {
        let cluster = offline_cluster();

        cluster
            .wait_network_ready()
            .await
            .expect("a never-started cluster must not fail readiness");
    }

    #[tokio::test]
    async fn common_control_returns_allocated_runner_endpoints() {
        let cluster = offline_cluster();
        let control: &dyn NodeControl = &cluster;

        let access = control
            .node_access("node-0")
            .await
            .expect("allocated access");

        assert_eq!(
            access.api_base_url().unwrap().as_str(),
            "http://127.0.0.1:1/"
        );
        assert_eq!(access.testing_port(), Some(2));
        assert_eq!(control.node_pid("node-0"), None);
        assert_eq!(control.node_names(), vec!["node-0"]);
        assert!(control.node_access("node-1").await.is_err());

        cluster.state.lock().unwrap().node_allocations[0] = None;
        let error = control.node_access("node-0").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no active port-forward allocation")
        );
    }

    #[tokio::test]
    async fn common_control_preserves_start_and_restart_validation() {
        let cluster = offline_cluster();
        let control: &dyn NodeControl = &cluster;
        let options = NodeLaunchOptions::default().with_persist_dir(PathBuf::from("/tmp/demo"));

        let error = control
            .start_node_with("node-0", options)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("persist/snapshot directories are not supported")
        );
        let error = control.restart_node("node-0").await.unwrap_err();
        assert!(error.to_string().contains("is not running"));

        cluster.closed.store(true, Ordering::Release);
        let error = control.node_access("node-0").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no longer owned by an active run")
        );
    }

    #[tokio::test]
    async fn node_control_reports_canonical_node_names() {
        let cluster = offline_cluster();

        assert_eq!(NodeControl::node_names(&cluster), vec!["node-0".to_owned()]);
    }

    #[tokio::test]
    async fn attachment_describes_namespace_and_node_service_selector() {
        let cluster = offline_cluster();

        assert_eq!(
            cluster.attachment(),
            ExistingCluster::for_k8s_selector_in_namespace(
                "ns".to_owned(),
                "app.kubernetes.io/instance=release".to_owned(),
            )
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleanup_guard_runs_once_and_locks_out_operations() {
        let cluster = Arc::new(offline_cluster());
        let guard = cluster.cleanup_guard();

        guard.cleanup();

        assert!(take_cleanup(&cluster.cleanup).is_none());
        assert!(cluster.closed.load(Ordering::Acquire));
        assert!(matches!(
            cluster.start_node("node-0").await,
            Err(ManualClusterError::Closed)
        ));
        assert!(matches!(
            cluster.stop_node("node-0").await,
            Err(ManualClusterError::Closed)
        ));
        assert!(matches!(
            cluster.wait_network_ready().await,
            Err(ManualClusterError::Closed)
        ));
        drop(cluster);
    }

    #[test]
    fn dummy_env_builds_cfgsync_override_artifacts_for_config_override() {
        let topology = testing_framework_core::topology::ClusterTopology::new(2);
        let options =
            StartNodeOptions::<DummyEnv>::default().with_config_override("override".to_owned());

        let artifacts = crate::env::build_cfgsync_override_artifacts::<DummyEnv>(
            &topology,
            1,
            &["node-0".to_owned(), "node-1".to_owned()],
            &options,
        )
        .expect("build override")
        .expect("expected override");

        assert_eq!(artifacts.files.len(), 1);
        assert_eq!(artifacts.files[0].content, "override");
    }
}
