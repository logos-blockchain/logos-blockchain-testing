//! Kind-gated smoke test for the app-model k8s path.
//!
//! Provisions a tiny kvstore cluster through `K8sClusterProvisioner` and
//! `ClusterApp` inside an app-host deploy context, then exercises the
//! backend-neutral `ClusterHandle` surface: node names, network readiness,
//! and a restart round-trip. The test runs only when
//! `K8S_RUNNER_REQUIRE_CLUSTER=1` (the same gate the example binaries use)
//! and exits immediately otherwise so offline `cargo test` stays green. It
//! reuses the `kvstore-node:local` image the Kind workflow builds and loads.

use std::io::Error as IoError;

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};
use testing_framework_app::{AppHostEnv, AppHostTopology, ClusterApp, DeployContext};
use testing_framework_core::{
    scenario::{
        Application, ClusterHandle, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView,
        DynError, NodeAccess, NodeClients, serialize_cluster_yaml_config,
    },
    topology::ClusterTopology,
};
use testing_framework_runner_k8s::{BinaryConfigK8sSpec, K8sBinaryApp, K8sClusterProvisioner};

const CONTAINER_CONFIG_PATH: &str = "/etc/kvstore/config.yaml";
const CONTAINER_HTTP_PORT: u16 = 8080;
const SERVICE_TESTING_PORT: u16 = 8081;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SmokePeerInfo {
    node_id: u64,
    http_address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SmokeNodeConfig {
    node_id: u64,
    http_port: u16,
    peers: Vec<SmokePeerInfo>,
    sync_interval_ms: u64,
}

/// Minimal test-only environment deploying the kvstore example image through
/// the standard binary+config k8s path.
struct SmokeEnv;

#[async_trait::async_trait]
impl Application for SmokeEnv {
    type Deployment = ClusterTopology;
    type NodeClient = String;
    type NodeConfig = SmokeNodeConfig;

    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Ok(access.api_base_url()?.to_string())
    }

    fn node_readiness_path() -> &'static str {
        "/health/ready"
    }
}

impl ClusterNodeConfigApplication for SmokeEnv {
    type ConfigError = IoError;

    fn static_network_port() -> u16 {
        CONTAINER_HTTP_PORT
    }

    fn build_cluster_node_config(
        node: &ClusterNodeView,
        peers: &[ClusterPeerView],
    ) -> Result<Self::NodeConfig, Self::ConfigError> {
        let peers = peers
            .iter()
            .map(|peer| SmokePeerInfo {
                node_id: peer.index() as u64,
                http_address: peer.authority(),
            })
            .collect();

        Ok(SmokeNodeConfig {
            node_id: node.index() as u64,
            http_port: node.network_port(),
            peers,
            sync_interval_ms: 500,
        })
    }

    fn serialize_cluster_node_config(
        config: &Self::NodeConfig,
    ) -> Result<String, Self::ConfigError> {
        serialize_cluster_yaml_config(config).map_err(IoError::other)
    }
}

impl K8sBinaryApp for SmokeEnv {
    fn k8s_binary_spec() -> BinaryConfigK8sSpec {
        BinaryConfigK8sSpec::conventional(
            "k8s-app-smoke",
            "smoke-node",
            "/usr/local/bin/kvstore-node",
            CONTAINER_CONFIG_PATH,
            CONTAINER_HTTP_PORT,
            SERVICE_TESTING_PORT,
        )
    }
}

fn cluster_required() -> bool {
    std::env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() == Ok("1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn app_model_smoke() -> Result<()> {
    if !cluster_required() {
        eprintln!(
            "skipping k8s app-model smoke test; set K8S_RUNNER_REQUIRE_CLUSTER=1 with a \
             reachable cluster to run it"
        );
        return Ok(());
    }

    let mut ctx = DeployContext::<AppHostEnv, _>::new_with_provisioner(
        AppHostTopology,
        NodeClients::default(),
        K8sClusterProvisioner,
    );

    let handle: ClusterHandle<SmokeEnv> = ctx
        .deploy(ClusterApp::<SmokeEnv>::new(ClusterTopology::new(2)))
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("deploying the smoke cluster through the app model")?;

    assert_eq!(
        handle.node_names(),
        vec!["node-0".to_owned(), "node-1".to_owned()]
    );

    handle
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for initial network readiness")?;

    handle
        .restart_node("node-0")
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("restarting node-0")?;

    handle
        .wait_node_ready("node-0")
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for node-0 readiness after restart")?;

    handle
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for network readiness after restart")?;

    assert!(
        handle.node_client("node-0").is_some(),
        "node-0 client must be rebuilt after the restart round-trip"
    );

    drop(handle);
    drop(ctx);
    Ok(())
}
