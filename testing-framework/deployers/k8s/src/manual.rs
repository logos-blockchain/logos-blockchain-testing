use std::{
    collections::HashSet,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
};

use cfgsync_core::Client as CfgsyncClient;
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use testing_framework_core::{
    manual::ManualClusterHandle,
    scenario::{
        ClusterWaitHandle, DynError, ExternalNodeSource, HttpReadinessRequirement, NodeClients,
        NodeControlHandle, StartNodeOptions, StartedNode,
    },
};
use thiserror::Error;
use tokio_retry::{RetryIf, strategy::FixedInterval};

use crate::{
    K8sDeployer,
    env::{
        K8sDeployEnv, build_cfgsync_override_artifacts, cfgsync_hostnames, cfgsync_service,
        cluster_identifiers, collect_port_specs, discovered_node_access, node_deployment_name,
        node_readiness_path, node_service_name, prepare_stack,
    },
    host::node_host,
    lifecycle::{
        cleanup::RunnerCleanup,
        wait::{
            ClusterWaitError, NodeConfigPorts, deployment::wait_for_deployment_ready,
            port_forward_service, ports::discover_node_ports,
        },
    },
};

#[derive(Debug, Error)]
pub enum ManualClusterError {
    #[error("kubernetes runner requires at least one node (nodes={nodes})")]
    UnsupportedTopology { nodes: usize },
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
}

struct ManualClusterState<E: K8sDeployEnv> {
    running: HashSet<usize>,
    node_clients: NodeClients<E>,
    known_clients: Vec<Option<E::NodeClient>>,
}

pub struct ManualCluster<E: K8sDeployEnv> {
    client: Client,
    namespace: String,
    release: String,
    topology: E::Deployment,
    node_count: usize,
    node_host: String,
    node_allocations: Vec<crate::wait::NodePortAllocation>,
    cleanup: Option<RunnerCleanup>,
    state: Arc<Mutex<ManualClusterState<E>>>,
}

impl<E: K8sDeployEnv> ManualCluster<E> {
    pub async fn from_topology(topology: E::Deployment) -> Result<Self, ManualClusterError> {
        let nodes = testing_framework_core::topology::DeploymentDescriptor::node_count(&topology);
        if nodes == 0 {
            return Err(ManualClusterError::UnsupportedTopology { nodes });
        }

        crate::ensure_rustls_provider_installed();
        let client = Client::try_default()
            .await
            .map_err(|source| ManualClusterError::ClientInit { source })?;
        let assets = prepare_stack::<E>(&topology, None)
            .map_err(|source| ManualClusterError::Assets { source })?;
        let (namespace, release) = cluster_identifiers::<E>();
        let cleanup = assets
            .install(&client, &namespace, &release, nodes)
            .await
            .map_err(|source| ManualClusterError::InstallStack { source })?;

        let node_ports = collect_port_specs::<E>(&topology).nodes;
        let node_allocations =
            discover_all_node_ports::<E>(&client, &namespace, &release, &node_ports).await?;
        scale_all_nodes::<E>(&client, &namespace, &release, nodes, 0).await?;

        Ok(Self {
            client,
            namespace,
            release,
            topology,
            node_count: nodes,
            node_host: node_host(),
            node_allocations,
            cleanup: Some(cleanup),
            state: Arc::new(Mutex::new(ManualClusterState {
                running: HashSet::new(),
                node_clients: NodeClients::default(),
                known_clients: vec![None; nodes],
            })),
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
        self.wait_node_ready(name).await?;
        let client = self.build_client(index, name)?;

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.running.insert(index);
        state.known_clients[index] = Some(client.clone());
        state.node_clients.add_node(client.clone());

        Ok(StartedNode {
            name: canonical_node_name(index),
            client,
        })
    }

    pub fn stop_all(&self) {
        block_on_best_effort(self.stop_all_async());
    }

    async fn stop_all_async(&self) -> Result<(), ManualClusterError> {
        let indices = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.running.iter().copied().collect::<Vec<_>>()
        };

        for index in indices {
            let name = canonical_node_name(index);
            self.stop_node(&name).await?;
        }

        Ok(())
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), ManualClusterError> {
        let index = self.require_running_node_index(name)?;
        scale_node::<E>(&self.client, &self.namespace, &self.release, index, 0).await?;
        scale_node::<E>(&self.client, &self.namespace, &self.release, index, 1).await?;
        self.wait_node_ready(name).await?;
        let client = self.build_client(index, name)?;

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.known_clients[index] = Some(client.clone());
        state.node_clients.add_node(client);
        Ok(())
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), ManualClusterError> {
        let index = self.require_running_node_index(name)?;
        scale_node::<E>(&self.client, &self.namespace, &self.release, index, 0).await?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.running.remove(&index);
        Ok(())
    }

    pub async fn wait_network_ready(&self) -> Result<(), ManualClusterError> {
        let running_ports = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .running
                .iter()
                .copied()
                .map(|index| self.node_allocations[index].api)
                .collect::<Vec<_>>()
        };

        if running_ports.is_empty() {
            return Ok(());
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
        let index = self.require_node_index(name)?;
        let port = self.node_allocations[index].api;
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

    pub fn add_external_sources(
        &self,
        external_sources: impl IntoIterator<Item = ExternalNodeSource>,
    ) -> Result<(), DynError> {
        let node_clients = self.node_clients();
        for source in external_sources {
            node_clients.add_node(E::external_node_client(&source)?);
        }
        Ok(())
    }

    pub fn add_external_clients(&self, clients: impl IntoIterator<Item = E::NodeClient>) {
        let node_clients = self.node_clients();
        for client in clients {
            node_clients.add_node(client);
        }
    }

    fn build_client(&self, index: usize, name: &str) -> Result<E::NodeClient, ManualClusterError> {
        let allocation = self.node_allocations[index];
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
        self.stop_all();
        if let Some(cleanup) = self.cleanup.take() {
            testing_framework_core::scenario::internal::CleanupGuard::cleanup(Box::new(cleanup));
        }
    }
}

#[async_trait::async_trait]
impl<E> NodeControlHandle<E> for ManualCluster<E>
where
    E: K8sDeployEnv,
{
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        Self::restart_node(self, name).await.map_err(Into::into)
    }

    async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        Self::start_node(self, name).await.map_err(Into::into)
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        Self::start_node_with(self, name, options)
            .await
            .map_err(Into::into)
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        Self::stop_node(self, name).await.map_err(Into::into)
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        Self::node_client(self, name)
    }
}

#[async_trait::async_trait]
impl<E> ClusterWaitHandle<E> for ManualCluster<E>
where
    E: K8sDeployEnv,
{
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        Self::wait_network_ready(self).await.map_err(Into::into)
    }
}

#[async_trait::async_trait]
impl<E> ManualClusterHandle<E> for ManualCluster<E> where E: K8sDeployEnv {}

impl<E> K8sDeployer<E>
where
    E: K8sDeployEnv,
{
    pub async fn manual_cluster_from_descriptors(
        &self,
        descriptors: E::Deployment,
    ) -> Result<ManualCluster<E>, ManualClusterError> {
        let _ = self;
        ManualCluster::from_topology(descriptors).await
    }
}

async fn discover_all_node_ports<E: K8sDeployEnv>(
    client: &Client,
    namespace: &str,
    release: &str,
    node_ports: &[NodeConfigPorts],
) -> Result<Vec<crate::wait::NodePortAllocation>, ManualClusterError> {
    let mut allocations = Vec::with_capacity(node_ports.len());
    for (index, ports) in node_ports.iter().enumerate() {
        let service_name = node_service_name::<E>(release, index);
        allocations.push(discover_node_ports(client, namespace, &service_name, *ports).await?);
    }
    Ok(allocations)
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

async fn scale_node<E: K8sDeployEnv>(
    client: &Client,
    namespace: &str,
    release: &str,
    index: usize,
    replicas: i32,
) -> Result<(), ManualClusterError> {
    let name = node_deployment_name::<E>(release, index);
    let deployments = Api::<Deployment>::namespaced(client.clone(), namespace);
    let patch = serde_json::json!({"spec": {"replicas": replicas}});
    deployments
        .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .map_err(|source| ManualClusterError::PatchDeployment {
            name: name.clone(),
            source,
        })?;

    wait_for_replicas(client, namespace, &name, replicas).await
}

async fn wait_for_replicas(
    client: &Client,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> Result<(), ManualClusterError> {
    if replicas > 0 {
        return wait_for_deployment_ready(client, namespace, name)
            .await
            .map_err(Into::into);
    }

    let deployments = Api::<Deployment>::namespaced(client.clone(), namespace);
    RetryIf::spawn(
        FixedInterval::from_millis(500).take(240),
        || async {
            let deployment = deployments.get(name).await.map_err(|source| {
                ManualClusterError::PatchDeployment {
                    name: name.to_owned(),
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
                    name: name.to_owned(),
                })
            }
        },
        |error: &ManualClusterError| matches!(error, ManualClusterError::NodeAlreadyRunning { .. }),
    )
    .await
}

fn validate_start_options<E: K8sDeployEnv>(
    options: &StartNodeOptions<E>,
) -> Result<(), ManualClusterError> {
    if options.persist_dir.is_some() || options.snapshot_dir.is_some() {
        return Err(ManualClusterError::UnsupportedStartOptions {
            message: "persist/snapshot directories are not supported".to_owned(),
        });
    }
    Ok(())
}

fn ensure_default_cfgsync_options<E: K8sDeployEnv>(
    options: &StartNodeOptions<E>,
) -> Result<(), ManualClusterError> {
    let default_peers = matches!(
        options.peers,
        testing_framework_core::scenario::PeerSelection::DefaultLayout
    );
    if default_peers && options.config_override.is_none() && options.config_patch.is_none() {
        return Ok(());
    }

    Err(ManualClusterError::UnsupportedStartOptions {
        message: "cfgsync override support is not configured for these start options".to_owned(),
    })
}

fn parse_node_index(name: &str) -> Option<usize> {
    name.strip_prefix("node-")?.parse().ok()
}

fn canonical_node_name(index: usize) -> String {
    format!("node-{index}")
}

fn block_on_best_effort(fut: impl std::future::Future<Output = Result<(), ManualClusterError>>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| {
            let _ = handle.block_on(fut);
        });
        return;
    }

    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        let _ = runtime.block_on(fut);
    }
}

#[cfg(test)]
mod tests {
    use testing_framework_core::{
        cfgsync::{StaticNodeConfigProvider, build_node_artifact_override},
        scenario::{
            Application, DefaultFeedRuntime, NodeAccess, NodeClients, PeerSelection,
            default_feed_result,
        },
    };

    use super::*;
    use crate::{
        RenderedHelmChartAssets, render_single_template_chart_assets, standard_port_specs,
    };

    struct DummyEnv;

    #[async_trait::async_trait]
    impl Application for DummyEnv {
        type Deployment = testing_framework_core::topology::ClusterTopology;
        type NodeClient = String;
        type NodeConfig = String;
        type FeedRuntime = DefaultFeedRuntime;

        fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
            Ok(access.api_base_url()?.to_string())
        }

        async fn prepare_feed(
            _node_clients: NodeClients<Self>,
        ) -> Result<
            (
                testing_framework_core::scenario::DefaultFeed,
                Self::FeedRuntime,
            ),
            DynError,
        > {
            default_feed_result()
        }
    }

    #[async_trait::async_trait]
    impl K8sDeployEnv for DummyEnv {
        fn k8s_runtime() -> crate::env::K8sRuntime<Self> {
            crate::env::K8sRuntime::new(crate::env::K8sInstall::new(
                |_topology| standard_port_specs(1, 8080, 8081),
                |_topology, _metrics_otlp_ingest_url| {
                    let assets: RenderedHelmChartAssets =
                        render_single_template_chart_assets("dummy", "dummy.yaml", "")?;
                    Ok(Box::new(assets) as Box<dyn crate::env::PreparedK8sStack>)
                },
            ))
            .with_manual(
                crate::env::K8sManual::new()
                    .with_cfgsync_service(|release| Some((format!("{release}-cfgsync"), 4400)))
                    .with_cfgsync_override_artifacts(|topology, node_index, hostnames, options| {
                        build_node_artifact_override::<Self>(
                            topology, node_index, hostnames, options,
                        )
                        .map_err(Into::into)
                    }),
            )
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
            let mut config = match &options.peers {
                PeerSelection::DefaultLayout => {
                    if options.config_override.is_none() && options.config_patch.is_none() {
                        return Ok(None);
                    }
                    format!("node={node_index};peers=default")
                }
                PeerSelection::None => format!("node={node_index};peers=none"),
                PeerSelection::Named(names) => {
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
