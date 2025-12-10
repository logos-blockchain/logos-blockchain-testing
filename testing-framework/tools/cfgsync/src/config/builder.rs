use std::{collections::HashMap, net::Ipv4Addr, str::FromStr as _};

use nomos_core::mantle::GenesisTx as _;
use nomos_libp2p::{Multiaddr, PeerId, ed25519};
use nomos_tracing_service::{LoggerLayer, MetricsLayer, TracingLayer, TracingSettings};
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use testing_framework_config::topology::configs::{
    GeneralConfig,
    api::GeneralApiConfig,
    blend::create_blend_configs,
    bootstrap::{SHORT_PROLONGED_BOOTSTRAP_PERIOD, create_bootstrap_configs},
    consensus::{ConsensusParams, create_consensus_configs, create_genesis_tx_with_declarations},
    da::{DaParams, create_da_configs},
    network::{NetworkParams, create_network_configs},
    time::default_time_config,
    tracing::GeneralTracingConfig,
    wallet::WalletConfig,
};

use crate::{
    config::{kms::create_kms_configs, providers::create_providers, validation::validate_inputs},
    host::{Host, HostKind, sort_hosts},
    network::rewrite_initial_peers,
};

#[must_use]
pub fn create_node_configs(
    consensus_params: &ConsensusParams,
    da_params: &DaParams,
    tracing_settings: &TracingSettings,
    wallet_config: &WalletConfig,
    ids: Option<Vec<[u8; 32]>>,
    da_ports: Option<Vec<u16>>,
    blend_ports: Option<Vec<u16>>,
    hosts: Vec<Host>,
) -> HashMap<Host, GeneralConfig> {
    let hosts = sort_hosts(hosts);

    validate_inputs(
        &hosts,
        consensus_params,
        ids.as_ref(),
        da_ports.as_ref(),
        blend_ports.as_ref(),
    )
    .expect("invalid cfgsync inputs");

    let ids = ids.unwrap_or_else(|| {
        let mut generated = vec![[0; 32]; consensus_params.n_participants];
        for id in &mut generated {
            thread_rng().fill(id);
        }
        generated
    });

    let ports = da_ports.unwrap_or_else(|| {
        (0..consensus_params.n_participants)
            .map(|_| get_available_udp_port().unwrap())
            .collect()
    });

    let blend_ports = blend_ports.unwrap_or_else(|| hosts.iter().map(|h| h.blend_port).collect());

    let mut consensus_configs = create_consensus_configs(&ids, consensus_params, wallet_config);
    let bootstrap_configs = create_bootstrap_configs(&ids, SHORT_PROLONGED_BOOTSTRAP_PERIOD);
    let da_configs = create_da_configs(&ids, da_params, &ports);
    let network_configs = create_network_configs(&ids, &NetworkParams::default());
    let blend_configs = create_blend_configs(&ids, &blend_ports);
    let api_configs = hosts
        .iter()
        .map(|host| GeneralApiConfig {
            address: format!("0.0.0.0:{}", host.api_port).parse().unwrap(),
            testing_http_address: format!("0.0.0.0:{}", host.testing_http_port)
                .parse()
                .unwrap(),
        })
        .collect::<Vec<_>>();
    let mut configured_hosts = HashMap::new();

    let initial_peer_templates: Vec<Vec<Multiaddr>> = network_configs
        .iter()
        .map(|cfg| cfg.backend.initial_peers.clone())
        .collect();
    let original_network_ports: Vec<u16> = network_configs
        .iter()
        .map(|cfg| cfg.backend.inner.port)
        .collect();
    let peer_ids: Vec<PeerId> = ids
        .iter()
        .map(|bytes| {
            let mut key_bytes = *bytes;
            let secret =
                ed25519::SecretKey::try_from_bytes(&mut key_bytes).expect("valid ed25519 key");
            PeerId::from_public_key(&ed25519::Keypair::from(secret).public().into())
        })
        .collect();

    let host_network_init_peers = rewrite_initial_peers(
        &initial_peer_templates,
        &original_network_ports,
        &hosts,
        &peer_ids,
    );

    let providers = create_providers(&hosts, &consensus_configs, &blend_configs, &da_configs);

    // Update genesis TX to contain Blend and DA providers.
    let ledger_tx = consensus_configs[0]
        .genesis_tx
        .mantle_tx()
        .ledger_tx
        .clone();
    let genesis_tx = create_genesis_tx_with_declarations(ledger_tx, providers);
    for c in &mut consensus_configs {
        c.genesis_tx = genesis_tx.clone();
    }

    // Set Blend and DA keys in KMS of each node config.
    let kms_configs = create_kms_configs(&blend_configs, &da_configs);

    for (i, host) in hosts.into_iter().enumerate() {
        let consensus_config = consensus_configs[i].clone();
        let api_config = api_configs[i].clone();

        // DA Libp2p network config.
        let mut da_config = da_configs[i].clone();
        da_config.listening_address = Multiaddr::from_str(&format!(
            "/ip4/0.0.0.0/udp/{}/quic-v1",
            host.da_network_port,
        ))
        .unwrap();
        if matches!(host.kind, HostKind::Validator) {
            da_config.policy_settings.min_dispersal_peers = 0;
        }

        // Libp2p network config.
        let mut network_config = network_configs[i].clone();
        network_config.backend.inner.host = Ipv4Addr::from_str("0.0.0.0").unwrap();
        network_config.backend.inner.port = host.network_port;
        network_config.backend.initial_peers = host_network_init_peers[i].clone();
        network_config.backend.inner.nat_config = nomos_libp2p::NatSettings::Static {
            external_address: Multiaddr::from_str(&format!(
                "/ip4/{}/udp/{}/quic-v1",
                host.ip, host.network_port
            ))
            .unwrap(),
        };

        // Blend network config.
        let mut blend_config = blend_configs[i].clone();
        blend_config.backend_core.listening_address =
            Multiaddr::from_str(&format!("/ip4/0.0.0.0/udp/{}/quic-v1", host.blend_port)).unwrap();

        // Tracing config.
        let tracing_config =
            update_tracing_identifier(tracing_settings.clone(), host.identifier.clone());

        // Time config
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

    configured_hosts
}

fn update_tracing_identifier(
    settings: TracingSettings,
    identifier: String,
) -> GeneralTracingConfig {
    GeneralTracingConfig {
        tracing_settings: TracingSettings {
            logger: match settings.logger {
                LoggerLayer::Loki(mut config) => {
                    config.host_identifier.clone_from(&identifier);
                    LoggerLayer::Loki(config)
                }
                other => other,
            },
            tracing: match settings.tracing {
                TracingLayer::Otlp(mut config) => {
                    config.service_name.clone_from(&identifier);
                    TracingLayer::Otlp(config)
                }
                other @ TracingLayer::None => other,
            },
            filter: settings.filter,
            metrics: match settings.metrics {
                MetricsLayer::Otlp(mut config) => {
                    config.host_identifier = identifier;
                    MetricsLayer::Otlp(config)
                }
                other @ MetricsLayer::None => other,
            },
            console: settings.console,
            level: settings.level,
        },
    }
}

#[cfg(test)]
mod cfgsync_tests {
    use std::{net::Ipv4Addr, num::NonZero, str::FromStr as _, time::Duration};

    use nomos_da_network_core::swarm::{
        DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
    };
    use nomos_libp2p::{Multiaddr, Protocol};
    use nomos_tracing_service::{
        ConsoleLayer, FilterLayer, LoggerLayer, MetricsLayer, TracingLayer, TracingSettings,
    };
    use testing_framework_config::topology::configs::{
        consensus::ConsensusParams, da::DaParams, wallet::WalletConfig,
    };
    use tracing::Level;

    use super::create_node_configs;
    use crate::host::{Host, HostKind};

    #[test]
    fn basic_ip_list() {
        let hosts = (0..10)
            .map(|i| Host {
                kind: HostKind::Validator,
                ip: Ipv4Addr::from_str(&format!("10.1.1.{i}")).unwrap(),
                identifier: "node".into(),
                network_port: 3000,
                da_network_port: 4044,
                blend_port: 5000,
                api_port: 18080,
                testing_http_port: 18081,
            })
            .collect();

        let configs = create_node_configs(
            &ConsensusParams {
                n_participants: 10,
                security_param: NonZero::new(10).unwrap(),
                active_slot_coeff: 0.9,
            },
            &DaParams {
                subnetwork_size: 2,
                dispersal_factor: 1,
                num_samples: 1,
                num_subnets: 2,
                old_blobs_check_interval: Duration::from_secs(5),
                blobs_validity_duration: Duration::from_secs(u64::MAX),
                global_params_path: String::new(),
                policy_settings: DAConnectionPolicySettings::default(),
                monitor_settings: DAConnectionMonitorSettings::default(),
                balancer_interval: Duration::ZERO,
                redial_cooldown: Duration::ZERO,
                replication_settings: ReplicationConfig {
                    seen_message_cache_size: 0,
                    seen_message_ttl: Duration::ZERO,
                },
                subnets_refresh_interval: Duration::from_secs(1),
                retry_shares_limit: 1,
                retry_commitments_limit: 1,
            },
            &TracingSettings {
                logger: LoggerLayer::None,
                tracing: TracingLayer::None,
                filter: FilterLayer::None,
                metrics: MetricsLayer::None,
                console: ConsoleLayer::None,
                level: Level::DEBUG,
            },
            &WalletConfig::default(),
            None,
            None,
            None,
            hosts,
        );

        for (host, config) in &configs {
            let network_port = config.network_config.backend.inner.port;
            let da_network_port = extract_port(&config.da_config.listening_address);
            let blend_port = extract_port(&config.blend_config.backend_core.listening_address);

            assert_eq!(network_port, host.network_port);
            assert_eq!(da_network_port, host.da_network_port);
            assert_eq!(blend_port, host.blend_port);
        }
    }

    fn extract_port(multiaddr: &Multiaddr) -> u16 {
        multiaddr
            .iter()
            .find_map(|protocol| match protocol {
                Protocol::Udp(port) => Some(port),
                _ => None,
            })
            .unwrap()
    }
}
