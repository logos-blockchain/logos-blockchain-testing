use std::io::Error;

use async_trait::async_trait;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use testing_framework_core::scenario::{
    Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
    NodeAccess,
};

pub const CLUSTER_PORT_KEY: &str = "cluster";

pub type NatsTopology = testing_framework_core::topology::ClusterTopology;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NatsNodeConfig {
    pub node_id: u64,
    pub client_port: u16,
    pub monitor_port: u16,
    pub cluster_port: u16,
    pub routes: Vec<String>,
}

#[derive(Clone)]
pub struct NatsClient {
    server_url: String,
    monitor_base_url: Url,
    http: reqwest::Client,
}

impl NatsClient {
    pub fn new(server_url: String, monitor_base_url: Url) -> Self {
        Self {
            server_url,
            monitor_base_url,
            http: reqwest::Client::new(),
        }
    }

    pub async fn connect(&self) -> Result<async_nats::Client, DynError> {
        Ok(async_nats::connect(&self.server_url).await?)
    }

    pub async fn is_healthy(&self) -> Result<bool, DynError> {
        let url = self.monitor_base_url.join("/healthz")?;
        let response = self.http.get(url).send().await?;
        Ok(response.status().is_success())
    }
}

pub struct NatsEnv;

#[async_trait]
impl Application for NatsEnv {
    type Deployment = NatsTopology;
    type NodeClient = NatsClient;
    type NodeConfig = NatsNodeConfig;

    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        let client_port = access.testing_port().unwrap_or(access.api_port());
        let server_url = format!("nats://{}:{client_port}", access.host());
        let monitor_base_url = access.api_base_url()?;
        Ok(NatsClient::new(server_url, monitor_base_url))
    }

    fn node_readiness_path() -> &'static str {
        "/healthz"
    }
}

impl ClusterNodeConfigApplication for NatsEnv {
    type ConfigError = Error;

    fn static_network_port() -> u16 {
        6222
    }

    fn static_named_ports() -> &'static [(&'static str, u16)] {
        &[("client", 4222), ("monitor", 8222)]
    }

    fn build_cluster_node_config(
        node: &ClusterNodeView,
        peers: &[ClusterPeerView],
    ) -> Result<Self::NodeConfig, Self::ConfigError> {
        let routes = peers
            .iter()
            .map(|peer| format!("nats-route://{}", peer.authority()))
            .collect::<Vec<_>>();

        Ok(NatsNodeConfig {
            node_id: node.index() as u64,
            client_port: node.require_named_port("client")?,
            monitor_port: node.require_named_port("monitor")?,
            cluster_port: node.network_port(),
            routes,
        })
    }

    fn serialize_cluster_node_config(
        config: &Self::NodeConfig,
    ) -> Result<String, Self::ConfigError> {
        Ok(render_nats_config(config))
    }
}

pub fn render_nats_config(config: &NatsNodeConfig) -> String {
    let routes = build_routes_line(&config.routes);

    format!(
        "server_name: node-{node_id}\nport: {client}\nhttp_port: {monitor}\njetstream {{\n  store_dir: \"jetstream\"\n}}\ncluster {{\n  name: tf\n  port: {cluster}\n{routes}}}\n",
        node_id = config.node_id,
        client = config.client_port,
        monitor = config.monitor_port,
        cluster = config.cluster_port,
        routes = routes,
    )
}

fn build_routes_line(routes: &[String]) -> String {
    if routes.is_empty() {
        return String::new();
    }

    format!("  routes: [{}]\n", routes.join(", "))
}
