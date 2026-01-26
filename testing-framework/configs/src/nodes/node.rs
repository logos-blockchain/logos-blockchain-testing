use nomos_node::{
    Config as NodeConfig, RocksBackendSettings, config::deployment::DeploymentSettings,
};
use nomos_sdp::SdpSettings;

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
pub fn create_node_config(config: GeneralConfig) -> NodeConfig {
    let network_config = config.network_config.clone();
    let (blend_user_config, blend_deployment, network_deployment) =
        build_blend_service_config(&config.blend_config);

    let deployment_settings =
        build_node_deployment_settings(&config, blend_deployment, network_deployment);

    NodeConfig {
        network: network_config,
        blend: blend_user_config,
        deployment: deployment_settings,
        cryptarchia: cryptarchia_config(&config),
        tracing: tracing_settings(&config),
        http: http_config(&config),
        storage: rocks_storage_settings(),
        time: time_config(&config),
        mempool: mempool_config(),
        sdp: SdpSettings { declaration: None },
        testing_http: testing_http_config(&config),
        wallet: wallet_settings(&config),
        key_management: config.kms_config.clone(),
    }
}

fn build_node_deployment_settings(
    config: &GeneralConfig,
    blend_deployment: nomos_node::config::blend::deployment::Settings,
    network_deployment: nomos_node::config::network::deployment::Settings,
) -> DeploymentSettings {
    DeploymentSettings::new_custom(
        blend_deployment,
        network_deployment,
        cryptarchia_deployment(config),
        time_deployment(config),
        mempool_deployment(),
    )
}

fn rocks_storage_settings() -> RocksBackendSettings {
    RocksBackendSettings {
        db_path: "./db".into(),
        read_only: false,
        column_family: Some("blocks".into()),
    }
}
