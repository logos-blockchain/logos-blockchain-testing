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
        Application, ClusterControlRequest, ClusterHandle, ClusterNodeConfigApplication,
        ClusterNodeView, ClusterPeerView, ClusterRequest, DynError, NodeAccess, NodeClients,
        ReadinessProbe, serialize_cluster_yaml_config,
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
struct SmokeEnv<const TCP: bool>;

#[async_trait::async_trait]
impl<const TCP: bool> Application for SmokeEnv<TCP> {
    type Deployment = ClusterTopology;
    type NodeClient = String;
    type NodeConfig = SmokeNodeConfig;

    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Ok(access.api_base_url()?.to_string())
    }

    fn node_readiness_probe() -> ReadinessProbe {
        if TCP {
            ReadinessProbe::Tcp
        } else {
            ReadinessProbe::Http {
                path: "/health/ready",
            }
        }
    }
}

impl<const TCP: bool> ClusterNodeConfigApplication for SmokeEnv<TCP> {
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

impl<const TCP: bool> K8sBinaryApp for SmokeEnv<TCP> {
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
    run_app_model_smoke::<false>().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn app_model_tcp_readiness() -> Result<()> {
    run_app_model_smoke::<true>().await
}

async fn run_app_model_smoke<const TCP: bool>() -> Result<()> {
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

    let handle: ClusterHandle<SmokeEnv<TCP>> = ctx
        .deploy(ClusterApp::<SmokeEnv<TCP>>::new(ClusterTopology::new(2)).with_name("alpha"))
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("deploying the smoke cluster through the app model")?;

    let sibling: ClusterHandle<SmokeEnv<TCP>> = ctx
        .deploy(ClusterApp::<SmokeEnv<TCP>>::new(ClusterTopology::new(1)).with_name("beta"))
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("deploying a sibling cluster into its own namespace")?;

    let alpha_namespace = handle
        .attachment()
        .and_then(|attachment| attachment.k8s_namespace())
        .ok_or_else(|| anyhow!("the alpha cluster must expose its Kubernetes namespace"))?;
    let beta_namespace = sibling
        .attachment()
        .and_then(|attachment| attachment.k8s_namespace())
        .ok_or_else(|| anyhow!("the beta cluster must expose its Kubernetes namespace"))?;
    assert!(
        alpha_namespace.ends_with("-alpha"),
        "the alpha cluster name must scope its namespace, got: {alpha_namespace}"
    );
    assert!(
        beta_namespace.ends_with("-beta"),
        "the beta cluster name must scope its namespace, got: {beta_namespace}"
    );
    assert_ne!(alpha_namespace, beta_namespace);

    assert_eq!(
        handle.node_names(),
        vec!["node-0".to_owned(), "node-1".to_owned()]
    );

    sibling
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for the sibling cluster's readiness")?;
    assert!(
        sibling.node_client("node-0").is_some(),
        "the sibling cluster must expose its own clients"
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

    sibling
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("the sibling cluster must stay reachable across the other cluster's restart")?;

    let attachment = handle.attachment().cloned().ok_or_else(|| {
        anyhow!("the managed cluster handle must expose an attachment descriptor")
    })?;

    let attached: ClusterHandle<SmokeEnv<TCP>> = ctx
        .deploy_cluster(
            ClusterRequest::attached(attachment).with_control(ClusterControlRequest::Full),
        )
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("attaching to the first cluster with full node control")?;

    assert!(
        !attached.clients().is_empty(),
        "the attached handle must expose node clients"
    );

    attached
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for network readiness through the attached handle")?;

    let attached_target = attached
        .node_names()
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("the attached handle must report its discovered node names"))?;

    attached
        .restart_node(&attached_target)
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("restarting a node through the attached handle")?;

    attached
        .wait_node_ready(&attached_target)
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for the restarted node through the attached handle")?;

    attached
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for network readiness after the attached restart")?;

    attached
        .stop_node(&attached_target)
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("stopping a node through the attached handle")?;

    let started = attached
        .start_node(&attached_target)
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("starting the stopped node through the attached handle")?;
    assert_eq!(
        started.name, attached_target,
        "the attached start must report the started node's name"
    );

    attached
        .wait_node_ready(&attached_target)
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for the node after the attached stop/start round-trip")?;

    attached
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("waiting for network readiness after the attached stop/start round-trip")?;

    handle
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("the managed handle must stay ready after the attached restart")?;

    drop(attached);

    handle
        .wait_network_ready()
        .await
        .map_err(|source| anyhow!(source.to_string()))
        .context("the original handle must stay ready after the attached handle is dropped")?;
    assert!(
        handle.node_client("node-0").is_some(),
        "the original handle must keep its node clients after the attached handle is dropped"
    );

    drop(sibling);
    drop(handle);
    drop(ctx);
    Ok(())
}
