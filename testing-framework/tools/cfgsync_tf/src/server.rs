use std::{fs, net::Ipv4Addr, num::NonZero, path::PathBuf, sync::Arc, time::Duration};

// DA Policy Constants
const DEFAULT_MAX_DISPERSAL_FAILURES: usize = 3;
const DEFAULT_MAX_SAMPLING_FAILURES: usize = 3;
const DEFAULT_MAX_REPLICATION_FAILURES: usize = 3;
const DEFAULT_MALICIOUS_THRESHOLD: usize = 10;

// DA Network Constants
const DEFAULT_SUBNETS_REFRESH_INTERVAL_SECS: u64 = 30;

// Bootstrap Constants
const DEFAULT_DELAY_BEFORE_NEW_DOWNLOAD_SECS: u64 = 10;
const DEFAULT_MAX_ORPHAN_CACHE_SIZE: usize = 5;

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use nomos_da_network_core::swarm::{
    DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
};
use nomos_tracing_service::TracingSettings;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, to_value};
use serde_with::serde_as;
use subnetworks_assignations::MembershipHandler;
use testing_framework_config::{
    nodes::{executor::create_executor_config, validator::create_validator_config},
    topology::configs::{consensus::ConsensusParams, da::DaParams, wallet::WalletConfig},
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
    pub da_ports: Option<Vec<u16>>,
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
    pub global_params_path: String,
    pub min_dispersal_peers: usize,
    pub min_replication_peers: usize,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub monitor_failure_time_window: Duration,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub balancer_interval: Duration,
    pub replication_settings: ReplicationConfig,
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
    pub fn to_da_params(&self) -> DaParams {
        DaParams {
            subnetwork_size: self.subnetwork_size,
            dispersal_factor: self.dispersal_factor,
            num_samples: self.num_samples,
            num_subnets: self.num_subnets,
            old_blobs_check_interval: self.old_blobs_check_interval,
            blobs_validity_duration: self.blobs_validity_duration,
            global_params_path: self.global_params_path.clone(),
            policy_settings: DAConnectionPolicySettings {
                min_dispersal_peers: self.min_dispersal_peers,
                min_replication_peers: self.min_replication_peers,
                max_dispersal_failures: DEFAULT_MAX_DISPERSAL_FAILURES,
                max_sampling_failures: DEFAULT_MAX_SAMPLING_FAILURES,
                max_replication_failures: DEFAULT_MAX_REPLICATION_FAILURES,
                malicious_threshold: DEFAULT_MALICIOUS_THRESHOLD,
            },
            monitor_settings: DAConnectionMonitorSettings {
                failure_time_window: self.monitor_failure_time_window,
                ..Default::default()
            },
            balancer_interval: self.balancer_interval,
            redial_cooldown: Duration::ZERO,
            replication_settings: self.replication_settings,
            subnets_refresh_interval: Duration::from_secs(DEFAULT_SUBNETS_REFRESH_INTERVAL_SECS),
            retry_shares_limit: self.retry_shares_limit,
            retry_commitments_limit: self.retry_commitments_limit,
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
    pub da_port: Option<u16>,
    #[serde(default)]
    pub blend_port: Option<u16>,
    #[serde(default)]
    pub api_port: Option<u16>,
    #[serde(default)]
    pub testing_http_port: Option<u16>,
}

async fn validator_config(
    State(config_repo): State<Arc<ConfigRepo>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let ClientIp {
        ip,
        identifier,
        network_port,
        da_port,
        blend_port,
        api_port,
        testing_http_port,
    } = payload;
    let ports = PortOverrides {
        network_port,
        da_network_port: da_port,
        blend_port,
        api_port,
        testing_http_port,
    };

    let (reply_tx, reply_rx) = channel();
    config_repo
        .register(Host::validator_from_ip(ip, identifier, ports), reply_tx)
        .await;

    (reply_rx.await).map_or_else(
        |_| (StatusCode::INTERNAL_SERVER_ERROR, "Error receiving config").into_response(),
        |config_response| match config_response {
            RepoResponse::Config(config) => {
                let config = create_validator_config(*config);
                let mut value = match to_value(&config) {
                    Ok(value) => value,
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("failed to serialize validator config: {err}"),
                        )
                            .into_response();
                    }
                };

                inject_defaults(&mut value);
                override_api_ports(&mut value, &ports);
                inject_da_assignations(&mut value, &config.da_network.membership);
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

async fn executor_config(
    State(config_repo): State<Arc<ConfigRepo>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let ClientIp {
        ip,
        identifier,
        network_port,
        da_port,
        blend_port,
        api_port,
        testing_http_port,
    } = payload;
    let ports = PortOverrides {
        network_port,
        da_network_port: da_port,
        blend_port,
        api_port,
        testing_http_port,
    };

    let (reply_tx, reply_rx) = channel();
    config_repo
        .register(Host::executor_from_ip(ip, identifier, ports), reply_tx)
        .await;

    (reply_rx.await).map_or_else(
        |_| (StatusCode::INTERNAL_SERVER_ERROR, "Error receiving config").into_response(),
        |config_response| match config_response {
            RepoResponse::Config(config) => {
                let config = create_executor_config(*config);
                let mut value = match to_value(&config) {
                    Ok(value) => value,
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("failed to serialize executor config: {err}"),
                        )
                            .into_response();
                    }
                };

                inject_defaults(&mut value);
                override_api_ports(&mut value, &ports);
                inject_da_assignations(&mut value, &config.da_network.membership);
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
        .route("/validator", post(validator_config))
        .route("/executor", post(executor_config))
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

fn inject_da_assignations(config: &mut Value, membership: &nomos_node::NomosDaMembership) {
    struct SubnetAssignment {
        subnet_id: String,
        peers: Vec<String>,
    }

    fn convert_subnet_to_assignment(
        subnet_id: impl ToString,
        members: impl IntoIterator<Item = impl ToString>,
    ) -> SubnetAssignment {
        let peers = members.into_iter().map(|peer| peer.to_string()).collect();

        SubnetAssignment {
            subnet_id: subnet_id.to_string(),
            peers,
        }
    }

    let subnetworks = membership.subnetworks();
    let assignations: std::collections::HashMap<String, Vec<String>> = subnetworks
        .into_iter()
        .map(|(subnet_id, members)| convert_subnet_to_assignment(subnet_id, members))
        .map(|assignment| (assignment.subnet_id, assignment.peers))
        .collect();

    if let Some(membership) = config.pointer_mut("/da_network/membership") {
        if let Some(map) = membership.as_object_mut() {
            map.insert("assignations".to_string(), json!(assignations));
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
