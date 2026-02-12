use std::{
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};

use lb_api_service::ApiServiceSettings;
use lb_chain_leader_service::LeaderWalletConfig;
use lb_chain_network::{BootstrapConfig as ChainBootstrapConfig, OrphanConfig, SyncConfig};
use lb_core::mantle::Value;
use lb_key_management_system_service::keys::{Key, secured_key::SecuredKey};
use lb_node::{
    api::backend::AxumBackendSettings as NodeAxumBackendSettings,
    config::{
        cryptarchia::{
            deployment::{
                SdpConfig as DeploymentSdpConfig, ServiceParameters,
                Settings as CryptarchiaDeploymentSettings,
            },
            serde::{
                Config as CryptarchiaConfig, LeaderConfig,
                NetworkConfig as CryptarchiaNetworkConfig,
                ServiceConfig as CryptarchiaServiceConfig,
            },
        },
        mempool::deployment::Settings as MempoolDeploymentSettings,
        time::{deployment::Settings as TimeDeploymentSettings, serde::Config as TimeConfig},
    },
};
use lb_wallet_service::WalletServiceSettings;
use time::OffsetDateTime;

use crate::{nodes::kms::key_id_for_preload_backend, timeouts, topology::configs::GeneralConfig};

// Configuration constants
const CRYPTARCHIA_GOSSIPSUB_PROTOCOL: &str = "/cryptarchia/proto";
const MEMPOOL_PUBSUB_TOPIC: &str = "mantle";
const STATE_RECORDING_INTERVAL_SECS: u64 = 60;
const IBD_DOWNLOAD_DELAY_SECS: u64 = 10;
const MAX_ORPHAN_CACHE_SIZE: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(5) };
const API_MAX_CONCURRENT_REQUESTS: usize = 1000;

static CHAIN_START_TIME: OnceLock<OffsetDateTime> = OnceLock::new();

fn get_or_init_chain_start_time() -> OffsetDateTime {
    *CHAIN_START_TIME.get_or_init(OffsetDateTime::now_utc)
}

pub(crate) fn cryptarchia_deployment(config: &GeneralConfig) -> CryptarchiaDeploymentSettings {
    let mantle_service_params = &config
        .consensus_config
        .ledger_config
        .sdp_config
        .service_params;
    let node_service_params = mantle_service_params
        .iter()
        .map(|(service_type, service_parameters)| {
            (
                service_type.clone(),
                ServiceParameters {
                    lock_period: service_parameters.lock_period,
                    inactivity_period: service_parameters.inactivity_period,
                    retention_period: service_parameters.retention_period,
                    timestamp: service_parameters.timestamp,
                },
            )
        })
        .collect::<HashMap<_, _>>();

    CryptarchiaDeploymentSettings {
        epoch_config: config.consensus_config.ledger_config.epoch_config,
        security_param: config
            .consensus_config
            .ledger_config
            .consensus_config
            .security_param(),
        sdp_config: DeploymentSdpConfig {
            service_params: node_service_params,
            min_stake: config.consensus_config.ledger_config.sdp_config.min_stake,
        },
        gossipsub_protocol: CRYPTARCHIA_GOSSIPSUB_PROTOCOL.to_owned(),
        genesis_state: config.consensus_config.genesis_tx.clone(),
    }
}

pub(crate) fn time_deployment(config: &GeneralConfig) -> TimeDeploymentSettings {
    TimeDeploymentSettings {
        slot_duration: config.time_config.slot_duration,
        chain_start_time: get_or_init_chain_start_time(),
    }
}

pub(crate) fn mempool_deployment() -> MempoolDeploymentSettings {
    MempoolDeploymentSettings {
        pubsub_topic: MEMPOOL_PUBSUB_TOPIC.to_owned(),
    }
}

pub(crate) fn cryptarchia_config(config: &GeneralConfig) -> CryptarchiaConfig {
    CryptarchiaConfig {
        service: CryptarchiaServiceConfig {
            recovery_file: PathBuf::from("recovery/cryptarchia.json"),
            bootstrap: lb_chain_service::BootstrapConfig {
                prolonged_bootstrap_period: config.bootstrapping_config.prolonged_bootstrap_period,
                force_bootstrap: false,
                offline_grace_period: lb_chain_service::OfflineGracePeriodConfig {
                    grace_period: timeouts::grace_period(),
                    state_recording_interval: Duration::from_secs(STATE_RECORDING_INTERVAL_SECS),
                },
            },
        },
        network: CryptarchiaNetworkConfig {
            bootstrap: ChainBootstrapConfig {
                ibd: lb_chain_network::IbdConfig {
                    peers: HashSet::new(),
                    delay_before_new_download: Duration::from_secs(IBD_DOWNLOAD_DELAY_SECS),
                },
            },
            sync: SyncConfig {
                orphan: OrphanConfig {
                    max_orphan_cache_size: MAX_ORPHAN_CACHE_SIZE,
                },
            },
        },
        leader: LeaderConfig {
            wallet: LeaderWalletConfig {
                max_tx_fee: Value::MAX,
                funding_pk: config.consensus_config.funding_sk.as_public_key(),
            },
        },
    }
}

pub(crate) fn time_config(config: &GeneralConfig) -> TimeConfig {
    TimeConfig {
        backend: lb_time_service::backends::NtpTimeBackendSettings {
            ntp_server: config.time_config.ntp_server.clone(),
            ntp_client_settings: lb_time_service::backends::ntp::async_client::NTPClientSettings {
                timeout: config.time_config.timeout,
                listening_interface: config.time_config.interface.clone(),
            },
            update_interval: config.time_config.update_interval,
        },
    }
}

pub(crate) fn mempool_config() -> lb_node::config::mempool::serde::Config {
    lb_node::config::mempool::serde::Config {
        recovery_path: PathBuf::from("recovery/mempool.json"),
    }
}

pub(crate) fn tracing_settings(config: &GeneralConfig) -> lb_tracing_service::TracingSettings {
    config.tracing_config.tracing_settings.clone()
}

pub(crate) fn http_config(config: &GeneralConfig) -> ApiServiceSettings<NodeAxumBackendSettings> {
    ApiServiceSettings {
        backend_settings: NodeAxumBackendSettings {
            address: config.api_config.address,
            max_concurrent_requests: API_MAX_CONCURRENT_REQUESTS,
            ..Default::default()
        },
    }
}

pub(crate) fn testing_http_config(
    config: &GeneralConfig,
) -> ApiServiceSettings<NodeAxumBackendSettings> {
    ApiServiceSettings {
        backend_settings: NodeAxumBackendSettings {
            address: config.api_config.testing_http_address,
            max_concurrent_requests: API_MAX_CONCURRENT_REQUESTS,
            ..Default::default()
        },
    }
}

pub(crate) fn wallet_settings(config: &GeneralConfig) -> WalletServiceSettings {
    wallet_settings_with_leader(config, true)
}

fn wallet_settings_with_leader(
    config: &GeneralConfig,
    include_leader: bool,
) -> WalletServiceSettings {
    let mut keys = HashMap::new();

    if include_leader {
        let leader_key = Key::Zk(config.consensus_config.leader_sk.clone().into());
        let leader_key_id = key_id_for_preload_backend(&leader_key);
        keys.insert(leader_key_id, config.consensus_config.leader_pk);
    }

    let funding_key = Key::Zk(config.consensus_config.funding_sk.clone());
    let funding_key_id = key_id_for_preload_backend(&funding_key);
    keys.insert(
        funding_key_id,
        config.consensus_config.funding_sk.to_public_key(),
    );

    // Note: wallet accounts are used by the transaction workload directly and
    // don't need to be registered for leader eligibility.

    let voucher_master_key_id =
        key_id_for_preload_backend(&Key::Zk(config.consensus_config.leader_sk.clone().into()));

    WalletServiceSettings {
        known_keys: keys,
        voucher_master_key_id,
        recovery_path: PathBuf::from("recovery/wallet.json"),
    }
}
