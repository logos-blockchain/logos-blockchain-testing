use std::time::Duration;

use nomos_da_dispersal::{
    DispersalServiceSettings,
    backend::kzgrs::{DispersalKZGRSBackendSettings, EncoderSettings},
};
use nomos_da_network_core::protocols::sampling::SubnetsConfig;
use nomos_da_network_service::{
    NetworkConfig as DaNetworkConfig,
    api::http::ApiAdapterSettings,
    backends::libp2p::{
        common::DaNetworkBackendSettings, executor::DaNetworkExecutorBackendSettings,
    },
};
use nomos_executor::config::Config as ExecutorConfig;
use nomos_node::{RocksBackendSettings, config::deployment::DeploymentSettings};
use nomos_sdp::SdpSettings;

use crate::{
    nodes::{
        blend::build_blend_service_config,
        common::{
            cryptarchia_config, cryptarchia_deployment, da_sampling_config, da_verifier_config,
            http_config, mempool_config, mempool_deployment, testing_http_config, time_config,
            time_deployment, tracing_settings, wallet_settings,
        },
    },
    topology::configs::GeneralConfig,
};

#[must_use]
pub fn create_executor_config(config: GeneralConfig) -> ExecutorConfig {
    let network_config = config.network_config.clone();
    let (blend_user_config, blend_deployment, network_deployment) =
        build_blend_service_config(&config.blend_config);

    let deployment_settings = DeploymentSettings::new_custom(
        blend_deployment,
        network_deployment,
        cryptarchia_deployment(&config),
        time_deployment(&config),
        mempool_deployment(),
    );

    ExecutorConfig {
        network: network_config,
        blend: blend_user_config,
        deployment: deployment_settings,
        cryptarchia: cryptarchia_config(&config),
        da_network: DaNetworkConfig {
            backend: DaNetworkExecutorBackendSettings {
                validator_settings: DaNetworkBackendSettings {
                    node_key: config.da_config.node_key.clone(),
                    listening_address: config.da_config.listening_address.clone(),
                    policy_settings: config.da_config.policy_settings.clone(),
                    monitor_settings: config.da_config.monitor_settings.clone(),
                    balancer_interval: config.da_config.balancer_interval,
                    redial_cooldown: config.da_config.redial_cooldown,
                    replication_settings: config.da_config.replication_settings,
                    subnets_settings: SubnetsConfig {
                        num_of_subnets: config.da_config.num_samples as usize,
                        shares_retry_limit: config.da_config.retry_shares_limit,
                        commitments_retry_limit: config.da_config.retry_commitments_limit,
                    },
                },
                num_subnets: config.da_config.num_subnets,
            },
            membership: config.da_config.membership.clone(),
            api_adapter_settings: ApiAdapterSettings {
                api_port: config.api_config.address.port(),
                is_secure: false,
            },
            subnet_refresh_interval: config.da_config.subnets_refresh_interval,
            subnet_threshold: config.da_config.num_samples as usize,
            min_session_members: config.da_config.num_samples as usize,
        },
        da_verifier: da_verifier_config(&config),
        tracing: tracing_settings(&config),
        http: http_config(&config),
        da_sampling: da_sampling_config(&config),
        storage: RocksBackendSettings {
            db_path: "./db".into(),
            read_only: false,
            column_family: Some("blocks".into()),
        },
        da_dispersal: DispersalServiceSettings {
            backend: DispersalKZGRSBackendSettings {
                encoder_settings: EncoderSettings {
                    num_columns: config.da_config.num_subnets as usize,
                    with_cache: false,
                    global_params_path: config.da_config.global_params_path.clone(),
                },
                dispersal_timeout: Duration::from_secs(20),
                retry_cooldown: Duration::from_secs(3),
                retry_limit: 2,
            },
        },
        time: time_config(&config),
        mempool: mempool_config(),
        sdp: SdpSettings { declaration: None },
        wallet: wallet_settings(&config),
        testing_http: testing_http_config(&config),
        key_management: config.kms_config,
    }
}
