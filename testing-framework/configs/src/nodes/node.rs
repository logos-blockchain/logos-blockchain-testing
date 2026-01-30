use key_management_system_service::keys::secured_key::SecuredKey as _;
use nomos_core::mantle::Value;
use nomos_node::{
    RocksBackendSettings, UserConfig,
    config::{RunConfig, deployment::DeploymentSettings},
};
use nomos_sdp::{SdpSettings, wallet::SdpWalletConfig};

use crate::{
    nodes::{
        blend::build_blend_service_config,
        common::{
            cryptarchia_config, cryptarchia_deployment, http_config, mempool_config,
            mempool_deployment, testing_http_config, time_config, time_deployment,
            tracing_settings, wallet_settings,
        },
    },
    topology::configs::GeneralConfig,
};

#[must_use]
pub fn create_node_config(config: GeneralConfig) -> RunConfig {
    let network_config = config.network_config.clone();
    let (blend_user_config, blend_deployment, network_deployment) =
        build_blend_service_config(&config.blend_config);

    let deployment_settings =
        build_node_deployment_settings(&config, blend_deployment, network_deployment);

    let user_settings = UserConfig {
        network: network_config,
        blend: blend_user_config,
        cryptarchia: cryptarchia_config(&config),
        tracing: tracing_settings(&config),
        http: http_config(&config),
        storage: rocks_storage_settings(),
        time: time_config(&config),
        mempool: mempool_config(),
        sdp: SdpSettings {
            declaration: None,
            wallet_config: SdpWalletConfig {
                max_tx_fee: Value::MAX,
                funding_pk: config.consensus_config.funding_sk.as_public_key(),
            },
        },
        testing_http: testing_http_config(&config),
        wallet: wallet_settings(&config),
        key_management: config.kms_config.clone(),
    };

    RunConfig {
        deployment: deployment_settings,
        user: user_settings,
    }
}

fn build_node_deployment_settings(
    config: &GeneralConfig,
    blend_deployment: nomos_node::config::blend::deployment::Settings,
    network_deployment: nomos_node::config::network::deployment::Settings,
) -> DeploymentSettings {
    DeploymentSettings {
        blend: blend_deployment,
        network: network_deployment,
        cryptarchia: cryptarchia_deployment(config),
        time: time_deployment(config),
        mempool: mempool_deployment(),
    }
}

fn rocks_storage_settings() -> RocksBackendSettings {
    RocksBackendSettings {
        db_path: "./db".into(),
        read_only: false,
        column_family: Some("blocks".into()),
    }
}
