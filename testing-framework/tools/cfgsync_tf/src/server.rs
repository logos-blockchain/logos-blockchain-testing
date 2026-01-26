use std::{fs, net::Ipv4Addr, num::NonZero, path::PathBuf, sync::Arc, time::Duration};

// Bootstrap Constants
const DEFAULT_DELAY_BEFORE_NEW_DOWNLOAD_SECS: u64 = 10;
const DEFAULT_MAX_ORPHAN_CACHE_SIZE: usize = 5;

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use nomos_tracing_service::TracingSettings;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, to_value};
use serde_with::serde_as;
use testing_framework_config::{
    nodes::node::create_node_config,
    topology::configs::{consensus::ConsensusParams, wallet::WalletConfig},
};
use tokio::sync::oneshot::channel;

use crate::{
    host::{Host, PortOverrides},
    repo::{ConfigRepo, RepoResponse},
};

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct CfgSyncConfig {
    pub port: u16,
    pub n_hosts: usize,
    pub timeout: u64,

    // ConsensusConfig related parameters
    pub security_param: NonZero<u32>,
    pub active_slot_coeff: f64,
    pub wallet: WalletConfig,
    #[serde(default)]
    pub ids: Option<Vec<[u8; 32]>>,
    #[serde(default)]
    pub blend_ports: Option<Vec<u16>>,

    // DaConfig related parameters
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

    // Tracing params
    pub tracing_settings: TracingSettings,
}

impl CfgSyncConfig {
    pub fn load_from_file(file_path: &PathBuf) -> Result<Self, String> {
        let config_content = fs::read_to_string(file_path)
            .map_err(|err| format!("Failed to read config file: {err}"))?;
        serde_yaml::from_str(&config_content)
            .map_err(|err| format!("Failed to parse config file: {err}"))
    }

    #[must_use]
    pub const fn to_consensus_params(&self) -> ConsensusParams {
        ConsensusParams {
            n_participants: self.n_hosts,
            security_param: self.security_param,
            active_slot_coeff: self.active_slot_coeff,
        }
    }

    #[must_use]
    pub fn to_tracing_settings(&self) -> TracingSettings {
        self.tracing_settings.clone()
    }

    #[must_use]
    pub fn wallet_config(&self) -> WalletConfig {
        self.wallet.clone()
    }
}

#[derive(Serialize, Deserialize)]
pub struct ClientIp {
    pub ip: Ipv4Addr,
    pub identifier: String,
    #[serde(default)]
    pub network_port: Option<u16>,
    #[serde(default)]
    pub blend_port: Option<u16>,
    #[serde(default)]
    pub api_port: Option<u16>,
    #[serde(default)]
    pub testing_http_port: Option<u16>,
}

async fn node_config(
    State(config_repo): State<Arc<ConfigRepo>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let ClientIp {
        ip,
        identifier,
        network_port,
        blend_port,
        api_port,
        testing_http_port,
    } = payload;
    let ports = PortOverrides {
        network_port,
        blend_port,
        api_port,
        testing_http_port,
    };

    let (reply_tx, reply_rx) = channel();
    config_repo
        .register(Host::node_from_ip(ip, identifier, ports), reply_tx)
        .await;

    (reply_rx.await).map_or_else(
        |_| (StatusCode::INTERNAL_SERVER_ERROR, "Error receiving config").into_response(),
        |config_response| match config_response {
            RepoResponse::Config(config) => {
                let config = create_node_config(*config);
                let mut value = match to_value(&config) {
                    Ok(value) => value,
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("failed to serialize node config: {err}"),
                        )
                            .into_response();
                    }
                };

                inject_defaults(&mut value);
                override_api_ports(&mut value, &ports);
                override_min_session_members(&mut value);

                (StatusCode::OK, Json(value)).into_response()
            }
            RepoResponse::Timeout => (StatusCode::REQUEST_TIMEOUT).into_response(),
            RepoResponse::Error(message) => {
                (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
            }
        },
    )
}

pub fn cfgsync_app(config_repo: Arc<ConfigRepo>) -> Router {
    Router::new()
        .route("/node", post(node_config))
        .with_state(config_repo)
}

fn override_api_ports(config: &mut Value, ports: &PortOverrides) {
    if let Some(api_port) = ports.api_port {
        if let Some(address) = config.pointer_mut("/http/backend_settings/address") {
            *address = json!(format!("0.0.0.0:{api_port}"));
        }
    }

    if let Some(testing_port) = ports.testing_http_port {
        if let Some(address) = config.pointer_mut("/testing_http/backend_settings/address") {
            *address = json!(format!("0.0.0.0:{testing_port}"));
        }
    }
}

fn override_min_session_members(config: &mut Value) {
    if let Some(value) = config.pointer_mut("/da_network/min_session_members") {
        *value = json!(1);
    }
}

fn inject_defaults(config: &mut Value) {
    if let Some(cryptarchia) = config
        .get_mut("cryptarchia")
        .and_then(|v| v.as_object_mut())
    {
        let bootstrap = cryptarchia.entry("bootstrap").or_insert_with(|| json!({}));
        if let Some(bootstrap_map) = bootstrap.as_object_mut() {
            bootstrap_map.entry("ibd").or_insert_with(
                || json!({ "peers": [], "delay_before_new_download": { "secs": DEFAULT_DELAY_BEFORE_NEW_DOWNLOAD_SECS, "nanos": 0 } }),
            );
        }

        cryptarchia
            .entry("network_adapter_settings")
            .or_insert_with(|| json!({ "topic": "/cryptarchia/proto" }));

        cryptarchia.entry("sync").or_insert_with(|| {
            json!({
                "orphan": { "max_orphan_cache_size": DEFAULT_MAX_ORPHAN_CACHE_SIZE }
            })
        });
    }
}
