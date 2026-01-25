use std::collections::HashMap;

use key_management_system_service::{backend::preload::PreloadKMSBackendSettings, keys::Key};
use nomos_libp2p::Multiaddr;

use crate::{
    node_address_from_port,
    nodes::kms::key_id_for_preload_backend,
    topology::configs::{
        GeneralConfig, GeneralConfigError, api, blend, bootstrap, consensus,
        consensus::{ConsensusParams, GeneralConsensusConfig},
        network,
        network::{Libp2pNetworkLayout, NetworkParams},
        time, tracing,
        wallet::WalletConfig,
    },
};

pub fn build_general_config_for_node(
    id: [u8; 32],
    network_port: u16,
    initial_peers: Vec<Multiaddr>,
    blend_port: u16,
    consensus_params: &ConsensusParams,
    wallet_config: &WalletConfig,
    base_consensus: &GeneralConsensusConfig,
    time_config: &time::GeneralTimeConfig,
) -> Result<GeneralConfig, GeneralConfigError> {
    let consensus_config =
        build_consensus_config_for_node(id, consensus_params, wallet_config, base_consensus)?;

    let bootstrap_config =
        bootstrap::create_bootstrap_configs(&[id], bootstrap::SHORT_PROLONGED_BOOTSTRAP_PERIOD)
            .into_iter()
            .next()
            .ok_or(GeneralConfigError::EmptyParticipants)?;

    let blend_config = blend::create_blend_configs(&[id], &[blend_port])
        .into_iter()
        .next()
        .ok_or(GeneralConfigError::EmptyParticipants)?;

    let network_config = network::build_network_config_for_node(id, network_port, initial_peers)?;

    let api_config = api::create_api_configs(&[id])?
        .into_iter()
        .next()
        .ok_or(GeneralConfigError::EmptyParticipants)?;

    let tracing_config = tracing::create_tracing_configs(&[id])
        .into_iter()
        .next()
        .ok_or(GeneralConfigError::EmptyParticipants)?;

    let kms_config = build_kms_config_for_node(&blend_config, wallet_config);

    Ok(GeneralConfig {
        consensus_config,
        bootstrapping_config: bootstrap_config,
        network_config,
        blend_config,
        api_config,
        tracing_config,
        time_config: time_config.clone(),
        kms_config,
    })
}

pub fn build_consensus_config_for_node(
    id: [u8; 32],
    consensus_params: &ConsensusParams,
    wallet_config: &WalletConfig,
    base: &GeneralConsensusConfig,
) -> Result<GeneralConsensusConfig, GeneralConfigError> {
    let mut config = consensus::create_consensus_configs(&[id], consensus_params, wallet_config)?
        .into_iter()
        .next()
        .ok_or(GeneralConfigError::EmptyParticipants)?;

    config.genesis_tx = base.genesis_tx.clone();
    config.utxos = base.utxos.clone();
    config.blend_notes = base.blend_notes.clone();
    config.wallet_accounts = base.wallet_accounts.clone();

    Ok(config)
}

pub fn build_initial_peers(network_params: &NetworkParams, peer_ports: &[u16]) -> Vec<Multiaddr> {
    match network_params.libp2p_network_layout {
        Libp2pNetworkLayout::Star => peer_ports
            .first()
            .map(|port| vec![node_address_from_port(*port)])
            .unwrap_or_default(),
        Libp2pNetworkLayout::Chain => peer_ports
            .last()
            .map(|port| vec![node_address_from_port(*port)])
            .unwrap_or_default(),
        Libp2pNetworkLayout::Full => peer_ports
            .iter()
            .map(|port| node_address_from_port(*port))
            .collect(),
    }
}

fn build_kms_config_for_node(
    blend_config: &blend::GeneralBlendConfig,
    wallet_config: &WalletConfig,
) -> PreloadKMSBackendSettings {
    let mut keys = HashMap::from([
        (
            key_id_for_preload_backend(&Key::Ed25519(blend_config.signer.clone())),
            Key::Ed25519(blend_config.signer.clone()),
        ),
        (
            key_id_for_preload_backend(&Key::Zk(blend_config.secret_zk_key.clone())),
            Key::Zk(blend_config.secret_zk_key.clone()),
        ),
    ]);

    for account in &wallet_config.accounts {
        let key = Key::Zk(account.secret_key.clone());
        let key_id = key_id_for_preload_backend(&key);
        keys.entry(key_id).or_insert(key);
    }

    PreloadKMSBackendSettings { keys }
}
