use std::{
    net::TcpListener,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use testing_framework_core::{
    scenario::{Application, DynError, ReadinessProbe, ReadinessRequirement},
    topology::DeploymentDescriptor,
};

use super::*;

static STABLE_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
struct DummyConfig;

#[derive(Clone)]
struct DummyTopology;

impl DeploymentDescriptor for DummyTopology {
    fn node_count(&self) -> usize {
        0
    }
}

struct DummyEnv;
struct TcpEnv;

#[async_trait::async_trait]
impl Application for DummyEnv {
    type Deployment = DummyTopology;
    type NodeClient = ();
    type NodeConfig = DummyConfig;
}

#[async_trait::async_trait]
impl Application for TcpEnv {
    type Deployment = DummyTopology;
    type NodeClient = ();
    type NodeConfig = DummyConfig;

    fn node_readiness_probe() -> ReadinessProbe {
        ReadinessProbe::Tcp
    }
}

#[async_trait::async_trait]
impl LocalDeployerEnv for DummyEnv {
    fn build_node_config(
        _context: crate::LocalBuildContext<'_, Self>,
    ) -> Result<BuiltNodeConfig<DummyConfig>, DynError> {
        build_dummy_node()
    }

    fn build_initial_node_configs(
        _topology: &Self::Deployment,
    ) -> Result<Vec<NodeConfigEntry<DummyConfig>>, crate::process::ProcessSpawnError> {
        build_dummy_initial_nodes()
    }

    async fn build_launch_spec(
        config: &DummyConfig,
        dir: &std::path::Path,
        label: &str,
    ) -> Result<crate::process::LaunchSpec, DynError> {
        build_dummy_launch_spec(config, dir, label)
    }

    fn node_endpoints(_config: &DummyConfig) -> Result<NodeEndpoints, DynError> {
        dummy_endpoints()
    }

    fn node_client(_endpoints: &NodeEndpoints) -> Result<Self::NodeClient, DynError> {
        Ok(())
    }

    async fn wait_readiness_stable(_nodes: &[Node<Self>]) -> Result<(), DynError> {
        dummy_wait_stable().await
    }
}

#[async_trait::async_trait]
impl LocalDeployerEnv for TcpEnv {
    fn build_node_config(
        _context: LocalBuildContext<'_, Self>,
    ) -> Result<BuiltNodeConfig<DummyConfig>, DynError> {
        unreachable!("readiness tests do not build node configs")
    }
}

fn build_dummy_node() -> Result<BuiltNodeConfig<DummyConfig>, DynError> {
    unreachable!("not used in this test")
}

fn build_dummy_initial_nodes()
-> Result<Vec<NodeConfigEntry<DummyConfig>>, crate::process::ProcessSpawnError> {
    unreachable!("not used in this test")
}

fn build_dummy_launch_spec(
    _config: &DummyConfig,
    _dir: &std::path::Path,
    _label: &str,
) -> Result<crate::process::LaunchSpec, DynError> {
    Ok(crate::process::LaunchSpec::default())
}

fn dummy_endpoints() -> Result<NodeEndpoints, DynError> {
    Ok(NodeEndpoints::default())
}

async fn dummy_wait_stable() -> Result<(), DynError> {
    STABLE_CALLS.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[tokio::test]
async fn empty_cluster_still_runs_stability_hook() {
    STABLE_CALLS.store(0, Ordering::SeqCst);
    let nodes: Vec<Node<DummyEnv>> = Vec::new();
    wait_local_readiness::<DummyEnv>(&nodes, ReadinessRequirement::AllNodesReady)
        .await
        .expect("empty cluster should be considered ready");
    assert_eq!(STABLE_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tcp_readiness_probe_accepts_bound_port() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind tcp listener");
    let port = listener.local_addr().expect("listener addr").port();

    wait_for_local_readiness_ports::<TcpEnv>(
        &[port],
        ReadinessRequirement::AllNodesReady,
        Some(Duration::from_secs(1)),
    )
    .await
    .expect("bound TCP port should be ready");
}

struct ConfigEnv;

#[derive(Clone)]
struct ConfigTopology(usize);

impl DeploymentDescriptor for ConfigTopology {
    fn node_count(&self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug)]
struct PreparedConfig {
    index: usize,
    network_port: u16,
    api_port: u16,
    peers: Vec<(usize, u16)>,
    value: String,
}

#[async_trait::async_trait]
impl Application for ConfigEnv {
    type Deployment = ConfigTopology;
    type NodeConfig = PreparedConfig;
    type NodeClient = ();
}

#[async_trait::async_trait]
impl LocalBinaryApp for ConfigEnv {
    fn initial_node_name_prefix() -> &'static str {
        "config"
    }

    fn initial_local_port_names() -> &'static [&'static str] {
        &["api"]
    }

    fn build_node_config(context: LocalBuildContext<'_, Self>) -> Result<PreparedConfig, DynError> {
        Ok(PreparedConfig {
            index: context.index,
            network_port: context.ports.network_port(),
            api_port: context.ports.require("api")?,
            peers: context
                .peers
                .iter()
                .map(|peer| (peer.index(), peer.network_port()))
                .collect(),
            value: context
                .template_config
                .map_or_else(|| "initial".into(), |config| config.value.clone()),
        })
    }

    fn local_process_spec() -> LocalProcessSpec {
        unreachable!("configuration tests do not launch processes")
    }

    fn render_local_config(_config: &PreparedConfig) -> Result<Vec<u8>, DynError> {
        unreachable!("configuration tests do not render files")
    }

    fn http_api_port(config: &PreparedConfig) -> u16 {
        config.api_port
    }
}

#[test]
fn initial_configs_receive_allocated_ports_and_other_nodes_as_peers() -> Result<(), DynError> {
    let nodes = ConfigEnv::build_initial_node_configs(&ConfigTopology(3))?;
    assert_eq!(nodes.len(), 3);
    for (index, node) in nodes.iter().enumerate() {
        assert_eq!(node.name, format!("config-{index}"));
        assert_eq!(node.config.index, index);
        assert_ne!(node.config.network_port, node.config.api_port);
        let expected = nodes
            .iter()
            .enumerate()
            .filter(|(peer, _)| *peer != index)
            .map(|(peer, node)| (peer, node.config.network_port))
            .collect::<Vec<_>>();
        assert_eq!(node.config.peers, expected);
        assert_eq!(node.config.value, "initial");
    }
    Ok(())
}

#[test]
fn individual_config_receives_template_and_current_peers() -> Result<(), DynError> {
    let topology = ConfigTopology(3);
    let mut nodes = ConfigEnv::build_initial_node_configs(&topology)?;
    nodes[1].config.value = "preserved".into();
    let peers = nodes
        .iter()
        .map(|node| node.config.network_port)
        .collect::<Vec<_>>();
    let built = build_node_from_template::<ConfigEnv>(
        &topology,
        3,
        &HashMap::new(),
        &StartNodeOptions::default(),
        &peers,
        Some(&nodes[1].config),
    )?;
    assert_eq!(built.config.value, "preserved");
    assert_eq!(built.config.index, 3);
    assert_eq!(
        built.config.peers,
        [(0, peers[0]), (1, peers[1]), (2, peers[2])]
    );
    assert_eq!(built.network_port, built.config.network_port);
    assert_ne!(built.config.network_port, built.config.api_port);
    Ok(())
}
