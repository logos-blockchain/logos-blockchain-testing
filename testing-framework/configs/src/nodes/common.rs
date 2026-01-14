use std::{collections::HashSet, num::NonZeroUsize, path::PathBuf, time::Duration};

use chain_leader::LeaderConfig as ChainLeaderConfig;
use chain_network::{BootstrapConfig as ChainBootstrapConfig, OrphanConfig, SyncConfig};
use chain_service::StartingState;
use nomos_api::ApiServiceSettings;
use nomos_da_sampling::{
    DaSamplingServiceSettings, backend::kzgrs::KzgrsSamplingBackendSettings,
    verifier::kzgrs::KzgrsDaVerifierSettings as SamplingVerifierSettings,
};
use nomos_da_verifier::{
    DaVerifierServiceSettings,
    backend::{kzgrs::KzgrsDaVerifierSettings, trigger::MempoolPublishTriggerConfig},
    storage::adapters::rocksdb::RocksAdapterSettings as VerifierStorageAdapterSettings,
};
use nomos_node::{
    api::backend::AxumBackendSettings as NodeAxumBackendSettings,
    config::{
        cryptarchia::{
            deployment::{
                SdpConfig as DeploymentSdpConfig, Settings as CryptarchiaDeploymentSettings,
            },
            serde::{
                Config as CryptarchiaConfig, NetworkConfig as CryptarchiaNetworkConfig,
                ServiceConfig as CryptarchiaServiceConfig,
            },
        },
        mempool::deployment::Settings as MempoolDeploymentSettings,
        time::{deployment::Settings as TimeDeploymentSettings, serde::Config as TimeConfig},
    },
};
use nomos_utils::math::NonNegativeF64;
use nomos_wallet::WalletServiceSettings;

use crate::{constants::KZG_PARAMS_FILENAME, timeouts, topology::configs::GeneralConfig};

// Configuration constants
const CRYPTARCHIA_GOSSIPSUB_PROTOCOL: &str = "/cryptarchia/proto";
const MEMPOOL_PUBSUB_TOPIC: &str = "mantle";
const STATE_RECORDING_INTERVAL_SECS: u64 = 60;
const IBD_DOWNLOAD_DELAY_SECS: u64 = 10;
const MAX_ORPHAN_CACHE_SIZE: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(5) };
const DA_PUBLISH_THRESHOLD: f64 = 0.8;
const API_RATE_LIMIT_PER_SECOND: u64 = 10000;
const API_RATE_LIMIT_BURST: u32 = 10000;
const API_MAX_CONCURRENT_REQUESTS: usize = 1000;
const BLOB_STORAGE_DIR: &str = "./";

pub(crate) fn cryptarchia_deployment(config: &GeneralConfig) -> CryptarchiaDeploymentSettings {
    CryptarchiaDeploymentSettings {
        epoch_config: config.consensus_config.ledger_config.epoch_config,
        consensus_config: config.consensus_config.ledger_config.consensus_config,
        sdp_config: DeploymentSdpConfig {
            service_params: config
                .consensus_config
                .ledger_config
                .sdp_config
                .service_params
                .clone(),
            min_stake: config.consensus_config.ledger_config.sdp_config.min_stake,
        },
        gossipsub_protocol: CRYPTARCHIA_GOSSIPSUB_PROTOCOL.to_owned(),
    }
}

pub(crate) fn time_deployment(config: &GeneralConfig) -> TimeDeploymentSettings {
    TimeDeploymentSettings {
        slot_duration: config.time_config.slot_duration,
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
            starting_state: StartingState::Genesis {
                genesis_tx: config.consensus_config.genesis_tx.clone(),
            },
            // Disable on-disk recovery in compose tests to avoid serde errors on
            // non-string keys and keep services alive.
            recovery_file: PathBuf::new(),
            bootstrap: chain_service::BootstrapConfig {
                prolonged_bootstrap_period: config.bootstrapping_config.prolonged_bootstrap_period,
                force_bootstrap: false,
                offline_grace_period: chain_service::OfflineGracePeriodConfig {
                    grace_period: timeouts::grace_period(),
                    state_recording_interval: Duration::from_secs(STATE_RECORDING_INTERVAL_SECS),
                },
            },
        },
        network: CryptarchiaNetworkConfig {
            bootstrap: ChainBootstrapConfig {
                ibd: chain_network::IbdConfig {
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
        leader: ChainLeaderConfig {
            pk: config.consensus_config.leader_config.pk,
            sk: config.consensus_config.leader_config.sk.clone(),
        },
    }
}

fn kzg_params_path(raw: &str) -> String {
    let path = PathBuf::from(raw);

    if path.is_dir() {
        return path.join(KZG_PARAMS_FILENAME).to_string_lossy().to_string();
    }

    path.to_string_lossy().to_string()
}

pub(crate) fn da_verifier_config(
    config: &GeneralConfig,
) -> DaVerifierServiceSettings<KzgrsDaVerifierSettings, (), (), VerifierStorageAdapterSettings> {
    let publish_threshold = match NonNegativeF64::try_from(DA_PUBLISH_THRESHOLD) {
        Ok(value) => value,
        Err(_) => unsafe {
            // Safety: `DA_PUBLISH_THRESHOLD` is a finite non-negative constant.
            std::hint::unreachable_unchecked()
        },
    };
    DaVerifierServiceSettings {
        share_verifier_settings: KzgrsDaVerifierSettings {
            global_params_path: kzg_params_path(&config.da_config.global_params_path),
            domain_size: config.da_config.num_subnets as usize,
        },
        tx_verifier_settings: (),
        network_adapter_settings: (),
        storage_adapter_settings: VerifierStorageAdapterSettings {
            blob_storage_directory: BLOB_STORAGE_DIR.into(),
        },
        mempool_trigger_settings: MempoolPublishTriggerConfig {
            publish_threshold,
            share_duration: timeouts::share_duration(),
            prune_duration: timeouts::prune_duration(),
            prune_interval: timeouts::prune_interval(),
        },
    }
}

pub(crate) fn da_sampling_config(
    config: &GeneralConfig,
) -> DaSamplingServiceSettings<KzgrsSamplingBackendSettings, SamplingVerifierSettings> {
    DaSamplingServiceSettings {
        sampling_settings: KzgrsSamplingBackendSettings {
            num_samples: config.da_config.num_samples,
            num_subnets: config.da_config.num_subnets,
            old_blobs_check_interval: config.da_config.old_blobs_check_interval,
            blobs_validity_duration: config.da_config.blobs_validity_duration,
        },
        share_verifier_settings: SamplingVerifierSettings {
            global_params_path: kzg_params_path(&config.da_config.global_params_path),
            domain_size: config.da_config.num_subnets as usize,
        },
        commitments_wait_duration: timeouts::commitments_wait(),
        sdp_blob_trigger_sampling_delay: crate::adjust_timeout(timeouts::sdp_trigger_delay()),
    }
}

pub(crate) fn time_config(config: &GeneralConfig) -> TimeConfig {
    TimeConfig {
        backend: nomos_time::backends::NtpTimeBackendSettings {
            ntp_server: config.time_config.ntp_server.clone(),
            ntp_client_settings: nomos_time::backends::ntp::async_client::NTPClientSettings {
                timeout: config.time_config.timeout,
                listening_interface: config.time_config.interface.clone(),
            },
            update_interval: config.time_config.update_interval,
        },
        chain_start_time: config.time_config.chain_start_time,
    }
}

pub(crate) fn mempool_config() -> nomos_node::config::mempool::serde::Config {
    nomos_node::config::mempool::serde::Config {
        // Disable mempool recovery for hermetic tests.
        recovery_path: PathBuf::new(),
    }
}

pub(crate) fn tracing_settings(config: &GeneralConfig) -> nomos_tracing_service::TracingSettings {
    config.tracing_config.tracing_settings.clone()
}

pub(crate) fn http_config(config: &GeneralConfig) -> ApiServiceSettings<NodeAxumBackendSettings> {
    ApiServiceSettings {
        backend_settings: NodeAxumBackendSettings {
            address: config.api_config.address,
            rate_limit_per_second: API_RATE_LIMIT_PER_SECOND,
            rate_limit_burst: API_RATE_LIMIT_BURST,
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
            rate_limit_per_second: API_RATE_LIMIT_PER_SECOND,
            rate_limit_burst: API_RATE_LIMIT_BURST,
            max_concurrent_requests: API_MAX_CONCURRENT_REQUESTS,
            ..Default::default()
        },
    }
}

pub(crate) fn wallet_settings(config: &GeneralConfig) -> WalletServiceSettings {
    WalletServiceSettings {
        known_keys: {
            let mut keys = HashSet::from_iter([config.consensus_config.leader_config.pk]);
            keys.extend(
                config
                    .consensus_config
                    .wallet_accounts
                    .iter()
                    .map(crate::topology::configs::wallet::WalletAccount::public_key),
            );
            keys
        },
    }
}
