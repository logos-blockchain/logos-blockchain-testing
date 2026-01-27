use std::{fs::File, num::NonZero, path::Path, time::Duration};

use anyhow::{Context as _, Result};
use nomos_tracing_service::TracingSettings;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tracing::debug;

use crate::topology::{configs::wallet::WalletConfig, generation::GeneratedTopology};

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncConfig {
    pub port: u16,
    pub n_hosts: usize,
    pub timeout: u64,
    pub security_param: NonZero<u32>,
    pub active_slot_coeff: f64,
    #[serde(default)]
    pub wallet: WalletConfig,
    #[serde(default)]
    pub ids: Option<Vec<[u8; 32]>>,
    #[serde(default)]
    pub blend_ports: Option<Vec<u16>>,
    pub subnetwork_size: usize,
    pub dispersal_factor: usize,
    pub num_samples: u16,
    pub num_subnets: u16,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub old_blobs_check_interval: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub blobs_validity_duration: Duration,
    pub min_dispersal_peers: usize,
    pub min_replication_peers: usize,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub monitor_failure_time_window: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub balancer_interval: Duration,
    pub retry_shares_limit: usize,
    pub retry_commitments_limit: usize,
    pub tracing_settings: TracingSettings,
}

pub fn load_cfgsync_template(path: &Path) -> Result<CfgSyncConfig> {
    debug!(path = %path.display(), "loading cfgsync template");
    let file = File::open(path)
        .with_context(|| format!("opening cfgsync template at {}", path.display()))?;
    serde_yaml::from_reader(file).context("parsing cfgsync template")
}

pub fn write_cfgsync_template(path: &Path, cfg: &CfgSyncConfig) -> Result<()> {
    debug!(path = %path.display(), "writing cfgsync template");
    let file = File::create(path)
        .with_context(|| format!("writing cfgsync template to {}", path.display()))?;
    let serializable = SerializableCfgSyncConfig::from(cfg);
    serde_yaml::to_writer(file, &serializable).context("serializing cfgsync template")
}

pub fn render_cfgsync_yaml(cfg: &CfgSyncConfig) -> Result<String> {
    debug!("rendering cfgsync yaml");
    let serializable = SerializableCfgSyncConfig::from(cfg);
    serde_yaml::to_string(&serializable).context("rendering cfgsync yaml")
}

pub fn apply_topology_overrides(cfg: &mut CfgSyncConfig, topology: &GeneratedTopology) {
    debug!(
        nodes = topology.nodes().len(),
        "applying topology overrides to cfgsync config"
    );
    let hosts = topology.nodes().len();
    cfg.n_hosts = hosts;

    let consensus = &topology.config().consensus_params;
    cfg.security_param = consensus.security_param;
    cfg.active_slot_coeff = consensus.active_slot_coeff;

    let config = topology.config();
    cfg.wallet = config.wallet_config.clone();
    cfg.ids = Some(topology.nodes().iter().map(|node| node.id).collect());
    cfg.blend_ports = Some(
        topology
            .nodes()
            .iter()
            .map(|node| node.blend_port)
            .collect(),
    );
}

#[serde_as]
#[derive(Serialize)]
struct SerializableCfgSyncConfig {
    port: u16,
    n_hosts: usize,
    timeout: u64,
    security_param: NonZero<u32>,
    active_slot_coeff: f64,
    wallet: WalletConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<Vec<[u8; 32]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blend_ports: Option<Vec<u16>>,
    subnetwork_size: usize,
    dispersal_factor: usize,
    num_samples: u16,
    num_subnets: u16,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    old_blobs_check_interval: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    blobs_validity_duration: Duration,
    min_dispersal_peers: usize,
    min_replication_peers: usize,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    monitor_failure_time_window: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    balancer_interval: Duration,
    retry_shares_limit: usize,
    retry_commitments_limit: usize,
    tracing_settings: TracingSettings,
}

impl From<&CfgSyncConfig> for SerializableCfgSyncConfig {
    fn from(cfg: &CfgSyncConfig) -> Self {
        Self {
            port: cfg.port,
            n_hosts: cfg.n_hosts,
            timeout: cfg.timeout,
            security_param: cfg.security_param,
            active_slot_coeff: cfg.active_slot_coeff,
            wallet: cfg.wallet.clone(),
            ids: cfg.ids.clone(),
            blend_ports: cfg.blend_ports.clone(),
            subnetwork_size: cfg.subnetwork_size,
            dispersal_factor: cfg.dispersal_factor,
            num_samples: cfg.num_samples,
            num_subnets: cfg.num_subnets,
            old_blobs_check_interval: cfg.old_blobs_check_interval,
            blobs_validity_duration: cfg.blobs_validity_duration,
            min_dispersal_peers: cfg.min_dispersal_peers,
            min_replication_peers: cfg.min_replication_peers,
            monitor_failure_time_window: cfg.monitor_failure_time_window,
            balancer_interval: cfg.balancer_interval,
            retry_shares_limit: cfg.retry_shares_limit,
            retry_commitments_limit: cfg.retry_commitments_limit,
            tracing_settings: cfg.tracing_settings.clone(),
        }
    }
}
