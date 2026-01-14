use std::{collections::HashMap, net::Ipv4Addr, str::FromStr as _};

use nomos_core::mantle::GenesisTx as _;
use nomos_libp2p::{Multiaddr, PeerId, ed25519};
use nomos_tracing_service::TracingSettings;
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use testing_framework_config::topology::configs::{
    GeneralConfig,
    api::GeneralApiConfig,
    base::{BaseConfigError, BaseConfigs, build_base_configs},
    consensus::{ConsensusConfigError, ConsensusParams, create_genesis_tx_with_declarations},
    da::DaParams,
    network::NetworkParams,
    time::default_time_config,
    wallet::WalletConfig,
};
use thiserror::Error;

use crate::{
    config::{
        kms::create_kms_configs,
        providers::{ProviderBuildError, try_create_providers},
        tracing::update_tracing_identifier,
        validation::{ValidationError, validate_inputs},
    },
    host::{Host, HostKind, sort_hosts},
    network::rewrite_initial_peers,
};

pub fn create_node_configs(
    consensus_params: &ConsensusParams,
    da_params: &DaParams,
    tracing_settings: &TracingSettings,
    wallet_config: &WalletConfig,
    ids: Option<Vec<[u8; 32]>>,
    da_ports: Option<Vec<u16>>,
    blend_ports: Option<Vec<u16>>,
    hosts: Vec<Host>,
) -> Result<HashMap<Host, GeneralConfig>, NodeConfigBuildError> {
    try_create_node_configs(
        consensus_params,
        da_params,
        tracing_settings,
        wallet_config,
        ids,
        da_ports,
        blend_ports,
        hosts,
    )
}

#[derive(Debug, Error)]
pub enum NodeConfigBuildError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Providers(#[from] ProviderBuildError),
    #[error(transparent)]
    Base(#[from] BaseConfigError),
    #[error(transparent)]
    Genesis(#[from] ConsensusConfigError),
    #[error("failed to allocate an available UDP port")]
    PortAllocFailed,
    #[error("failed to parse multiaddr '{value}': {message}")]
    InvalidMultiaddr { value: String, message: String },
    #[error("failed to parse socket addr '{value}': {source}")]
    InvalidSocketAddr {
        value: String,
        source: std::net::AddrParseError,
    },
    #[error("failed to decode ed25519 secret key bytes")]
    InvalidEd25519SecretKey,
    #[error("config generation requires at least one consensus config")]
    MissingConsensusConfig,
    #[error("host/config length mismatch")]
    HostConfigLenMismatch,
    #[error(transparent)]
    PeerRewrite(#[from] crate::network::peers::PeerRewriteError),
}

pub fn try_create_node_configs(
    consensus_params: &ConsensusParams,
    da_params: &DaParams,
    tracing_settings: &TracingSettings,
    wallet_config: &WalletConfig,
    ids: Option<Vec<[u8; 32]>>,
    da_ports: Option<Vec<u16>>,
    blend_ports: Option<Vec<u16>>,
    hosts: Vec<Host>,
) -> Result<HashMap<Host, GeneralConfig>, NodeConfigBuildError> {
    let hosts = sort_hosts(hosts);

    validate_inputs(
        &hosts,
        consensus_params,
        ids.as_ref(),
        da_ports.as_ref(),
        blend_ports.as_ref(),
    )?;

    let ids = generate_ids(consensus_params.n_participants, ids);
    let ports = resolve_da_ports(consensus_params.n_participants, da_ports)?;
    let blend_ports = resolve_blend_ports(&hosts, blend_ports);

    let BaseConfigs {
        mut consensus_configs,
        bootstrap_configs,
        da_configs,
        network_configs,
        blend_configs,
    } = build_base_configs(
        &ids,
        consensus_params,
        da_params,
        &NetworkParams::default(),
        wallet_config,
        &ports,
        &blend_ports,
    )?;

    let api_configs = build_api_configs(&hosts)?;
    let mut configured_hosts = HashMap::new();

    let initial_peer_templates: Vec<Vec<Multiaddr>> = network_configs
        .iter()
        .map(|cfg| cfg.backend.initial_peers.clone())
        .collect();
    let original_network_ports: Vec<u16> = network_configs
        .iter()
        .map(|cfg| cfg.backend.swarm.port)
        .collect();
    let peer_ids = build_peer_ids(&ids)?;

    let host_network_init_peers = rewrite_initial_peers(
        &initial_peer_templates,
        &original_network_ports,
        &hosts,
        &peer_ids,
    )?;

    let providers = try_create_providers(&hosts, &consensus_configs, &blend_configs, &da_configs)?;

    let first_consensus = consensus_configs
        .get(0)
        .ok_or(NodeConfigBuildError::MissingConsensusConfig)?;
    let ledger_tx = first_consensus.genesis_tx.mantle_tx().ledger_tx.clone();
    let genesis_tx = create_genesis_tx_with_declarations(ledger_tx, providers)?;

    for c in &mut consensus_configs {
        c.genesis_tx = genesis_tx.clone();
    }

    let kms_configs = create_kms_configs(&blend_configs, &da_configs);

    for (i, host) in hosts.into_iter().enumerate() {
        if i >= consensus_configs.len()
            || i >= api_configs.len()
            || i >= da_configs.len()
            || i >= network_configs.len()
            || i >= blend_configs.len()
            || i >= host_network_init_peers.len()
            || i >= kms_configs.len()
            || i >= bootstrap_configs.len()
        {
            return Err(NodeConfigBuildError::HostConfigLenMismatch);
        }

        let consensus_config = consensus_configs[i].clone();
        let api_config = api_configs[i].clone();

        let mut da_config = da_configs[i].clone();
        let da_addr_value = format!("/ip4/0.0.0.0/udp/{}/quic-v1", host.da_network_port);
        da_config.listening_address = Multiaddr::from_str(&da_addr_value).map_err(|source| {
            NodeConfigBuildError::InvalidMultiaddr {
                value: da_addr_value,
                message: source.to_string(),
            }
        })?;
        if matches!(host.kind, HostKind::Validator) {
            da_config.policy_settings.min_dispersal_peers = 0;
        }

        let mut network_config = network_configs[i].clone();
        network_config.backend.swarm.host = Ipv4Addr::UNSPECIFIED;
        network_config.backend.swarm.port = host.network_port;
        network_config.backend.initial_peers = host_network_init_peers[i].clone();
        let nat_value = format!("/ip4/{}/udp/{}/quic-v1", host.ip, host.network_port);
        let nat_addr = Multiaddr::from_str(&nat_value).map_err(|source| {
            NodeConfigBuildError::InvalidMultiaddr {
                value: nat_value,
                message: source.to_string(),
            }
        })?;
        network_config.backend.swarm.nat_config = nomos_libp2p::NatSettings::Static {
            external_address: nat_addr,
        };

        let mut blend_config = blend_configs[i].clone();
        let blend_value = format!("/ip4/0.0.0.0/udp/{}/quic-v1", host.blend_port);
        blend_config.backend_core.listening_address =
            Multiaddr::from_str(&blend_value).map_err(|source| {
                NodeConfigBuildError::InvalidMultiaddr {
                    value: blend_value,
                    message: source.to_string(),
                }
            })?;

        let tracing_config =
            update_tracing_identifier(tracing_settings.clone(), host.identifier.clone());
        let time_config = default_time_config();

        configured_hosts.insert(
            host.clone(),
            GeneralConfig {
                consensus_config,
                bootstrapping_config: bootstrap_configs[i].clone(),
                da_config,
                network_config,
                blend_config,
                api_config,
                tracing_config,
                time_config,
                kms_config: kms_configs[i].clone(),
            },
        );
    }

    Ok(configured_hosts)
}

fn generate_ids(count: usize, ids: Option<Vec<[u8; 32]>>) -> Vec<[u8; 32]> {
    ids.unwrap_or_else(|| {
        let mut generated = vec![[0; 32]; count];

        for id in &mut generated {
            thread_rng().fill(id);
        }

        generated
    })
}

fn resolve_da_ports(
    count: usize,
    da_ports: Option<Vec<u16>>,
) -> Result<Vec<u16>, NodeConfigBuildError> {
    da_ports.map(Ok).unwrap_or_else(|| {
        (0..count)
            .map(|_| get_available_udp_port().ok_or(NodeConfigBuildError::PortAllocFailed))
            .collect()
    })
}

fn resolve_blend_ports(hosts: &[Host], blend_ports: Option<Vec<u16>>) -> Vec<u16> {
    blend_ports.unwrap_or_else(|| hosts.iter().map(|h| h.blend_port).collect())
}

fn build_api_configs(hosts: &[Host]) -> Result<Vec<GeneralApiConfig>, NodeConfigBuildError> {
    hosts
        .iter()
        .map(|host| {
            let address_value = format!("0.0.0.0:{}", host.api_port);
            let testing_value = format!("0.0.0.0:{}", host.testing_http_port);
            Ok(GeneralApiConfig {
                address: address_value.parse().map_err(|source| {
                    NodeConfigBuildError::InvalidSocketAddr {
                        value: address_value,
                        source,
                    }
                })?,
                testing_http_address: testing_value.parse().map_err(|source| {
                    NodeConfigBuildError::InvalidSocketAddr {
                        value: testing_value,
                        source,
                    }
                })?,
            })
        })
        .collect()
}

fn build_peer_ids(ids: &[[u8; 32]]) -> Result<Vec<PeerId>, NodeConfigBuildError> {
    ids.iter()
        .map(|bytes| {
            let mut key_bytes = *bytes;
            let secret = ed25519::SecretKey::try_from_bytes(&mut key_bytes)
                .map_err(|_| NodeConfigBuildError::InvalidEd25519SecretKey)?;
            Ok(PeerId::from_public_key(
                &ed25519::Keypair::from(secret).public().into(),
            ))
        })
        .collect()
}
