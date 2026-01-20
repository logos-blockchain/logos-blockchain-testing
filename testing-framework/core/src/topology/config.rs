use std::time::Duration;

use nomos_core::{
    mantle::GenesisTx as _,
    sdp::{Locator, ServiceType},
};
use testing_framework_config::topology::{
    configs::{
        api::{ApiConfigError, create_api_configs},
        base::{BaseConfigError, BaseConfigs, build_base_configs},
        consensus::{
            ConsensusConfigError, ConsensusParams, ProviderInfo,
            create_genesis_tx_with_declarations,
        },
        da::DaParams,
        network::{Libp2pNetworkLayout, NetworkParams},
        tracing::create_tracing_configs,
        wallet::WalletConfig,
    },
    invariants::{TopologyInvariantError, validate_generated_vectors},
};
use thiserror::Error;

use crate::topology::{
    configs::{GeneralConfig, time::default_time_config},
    generation::{GeneratedNodeConfig, GeneratedTopology},
    utils::{TopologyResolveError, create_kms_configs, resolve_ids, resolve_ports},
};

const DEFAULT_DA_BALANCER_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
pub enum TopologyBuildError {
    #[error("topology must include at least one node")]
    EmptyParticipants,
    #[error(transparent)]
    Invariants(#[from] TopologyInvariantError),
    #[error(transparent)]
    Resolve(#[from] TopologyResolveError),
    #[error(transparent)]
    Base(#[from] BaseConfigError),
    #[error(transparent)]
    Api(#[from] ApiConfigError),
    #[error(transparent)]
    Genesis(#[from] ConsensusConfigError),
    #[error("config generation requires at least one consensus config")]
    MissingConsensusConfig,
    #[error("internal config vector mismatch for {label} (expected {expected}, got {actual})")]
    VectorLenMismatch {
        label: &'static str,
        expected: usize,
        actual: usize,
    },
}

/// High-level topology settings used to generate node configs for a scenario.
#[derive(Clone)]
pub struct TopologyConfig {
    pub n_nodes: usize,
    pub consensus_params: ConsensusParams,
    pub da_params: DaParams,
    pub network_params: NetworkParams,
    pub wallet_config: WalletConfig,
}

impl TopologyConfig {
    /// Create a config with zero nodes; counts must be set before building.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            n_nodes: 0,
            consensus_params: ConsensusParams::default_for_participants(1),
            da_params: DaParams::default(),
            network_params: NetworkParams::default(),
            wallet_config: WalletConfig::default(),
        }
    }

    #[must_use]
    /// Convenience config with two nodes for consensus-only scenarios.
    pub fn two_nodes() -> Self {
        Self {
            n_nodes: 2,
            consensus_params: ConsensusParams::default_for_participants(2),
            da_params: DaParams::default(),
            network_params: NetworkParams::default(),
            wallet_config: WalletConfig::default(),
        }
    }

    #[must_use]
    /// Build a topology with explicit node count.
    pub fn with_node_count(nodes: usize) -> Self {
        let participants = nodes;

        let mut da_params = DaParams::default();
        let da_nodes = participants;
        if da_nodes <= 1 {
            da_params.subnetwork_size = 1;
            da_params.num_subnets = 1;
            da_params.dispersal_factor = 1;
            da_params.policy_settings.min_dispersal_peers = 0;
            da_params.policy_settings.min_replication_peers = 0;
        } else {
            let dispersal = da_nodes.min(da_params.dispersal_factor.max(2));
            da_params.dispersal_factor = dispersal;
            da_params.subnetwork_size = da_params.subnetwork_size.max(dispersal);
            da_params.num_subnets = da_params.subnetwork_size as u16;
            let min_peers = dispersal.saturating_sub(1).max(1);
            da_params.policy_settings.min_dispersal_peers = min_peers;
            da_params.policy_settings.min_replication_peers = min_peers;
            da_params.balancer_interval = DEFAULT_DA_BALANCER_INTERVAL;
        }

        Self {
            n_nodes: nodes,
            consensus_params: ConsensusParams::default_for_participants(participants),
            da_params,
            network_params: NetworkParams::default(),
            wallet_config: WalletConfig::default(),
        }
    }

    #[must_use]
    pub const fn wallet(&self) -> &WalletConfig {
        &self.wallet_config
    }
}

/// Builder that produces `GeneratedTopology` instances from a `TopologyConfig`.
#[derive(Clone)]
pub struct TopologyBuilder {
    config: TopologyConfig,
    ids: Option<Vec<[u8; 32]>>,
    da_ports: Option<Vec<u16>>,
    blend_ports: Option<Vec<u16>>,
}

impl TopologyBuilder {
    #[must_use]
    /// Create a builder from a base topology config.
    pub const fn new(config: TopologyConfig) -> Self {
        Self {
            config,
            ids: None,
            da_ports: None,
            blend_ports: None,
        }
    }

    #[must_use]
    /// Provide deterministic node IDs.
    pub fn with_ids(mut self, ids: Vec<[u8; 32]>) -> Self {
        self.ids = Some(ids);
        self
    }

    #[must_use]
    /// Override DA ports for nodes in order.
    pub fn with_da_ports(mut self, ports: Vec<u16>) -> Self {
        self.da_ports = Some(ports);
        self
    }

    #[must_use]
    /// Override blend ports for nodes in order.
    pub fn with_blend_ports(mut self, ports: Vec<u16>) -> Self {
        self.blend_ports = Some(ports);
        self
    }

    #[must_use]
    /// Set total node count.
    pub const fn with_node_count(mut self, nodes: usize) -> Self {
        self.config.n_nodes = nodes;
        self
    }

    #[must_use]
    /// Configure the libp2p network layout.
    pub const fn with_network_layout(mut self, layout: Libp2pNetworkLayout) -> Self {
        self.config.network_params.libp2p_network_layout = layout;
        self
    }

    /// Override wallet configuration used in genesis.
    pub fn with_wallet_config(mut self, wallet: WalletConfig) -> Self {
        self.config.wallet_config = wallet;
        self
    }

    /// Finalize and generate topology and node descriptors.
    pub fn build(self) -> Result<GeneratedTopology, TopologyBuildError> {
        let Self {
            config,
            ids,
            da_ports,
            blend_ports,
        } = self;

        let n_participants = participant_count(&config)?;

        let (ids, da_ports, blend_ports) =
            resolve_and_validate_vectors(ids, da_ports, blend_ports, n_participants)?;

        let BaseConfigs {
            mut consensus_configs,
            bootstrap_configs: bootstrapping_config,
            da_configs,
            network_configs,
            blend_configs,
        } = build_base_configs(
            &ids,
            &config.consensus_params,
            &config.da_params,
            &config.network_params,
            &config.wallet_config,
            &da_ports,
            &blend_ports,
        )?;

        let api_configs = create_api_configs(&ids)?;
        let tracing_configs = create_tracing_configs(&ids);
        let time_config = default_time_config();

        let first_consensus = consensus_configs
            .first()
            .ok_or(TopologyBuildError::MissingConsensusConfig)?;
        let providers = collect_provider_infos(first_consensus, &da_configs, &blend_configs)?;

        let genesis_tx = create_consensus_genesis_tx(first_consensus, providers)?;
        apply_consensus_genesis_tx(&mut consensus_configs, &genesis_tx);

        let kms_configs =
            create_kms_configs(&blend_configs, &da_configs, &config.wallet_config.accounts);

        let nodes = build_node_descriptors(
            &config,
            n_participants,
            &ids,
            &da_ports,
            &blend_ports,
            &consensus_configs,
            &bootstrapping_config,
            &da_configs,
            &network_configs,
            &blend_configs,
            &api_configs,
            &tracing_configs,
            &kms_configs,
            &time_config,
        )?;

        Ok(GeneratedTopology { config, nodes })
    }

    #[must_use]
    pub const fn config(&self) -> &TopologyConfig {
        &self.config
    }
}

fn participant_count(config: &TopologyConfig) -> Result<usize, TopologyBuildError> {
    let n_participants = config.n_nodes;
    if n_participants == 0 {
        return Err(TopologyBuildError::EmptyParticipants);
    }

    Ok(n_participants)
}

fn resolve_and_validate_vectors(
    ids: Option<Vec<[u8; 32]>>,
    da_ports: Option<Vec<u16>>,
    blend_ports: Option<Vec<u16>>,
    n_participants: usize,
) -> Result<(Vec<[u8; 32]>, Vec<u16>, Vec<u16>), TopologyBuildError> {
    let ids = resolve_ids(ids, n_participants)?;
    let da_ports = resolve_ports(da_ports, n_participants, "DA")?;
    let blend_ports = resolve_ports(blend_ports, n_participants, "Blend")?;

    validate_generated_vectors(n_participants, &ids, &da_ports, &blend_ports)?;

    Ok((ids, da_ports, blend_ports))
}

fn collect_provider_infos(
    first_consensus: &testing_framework_config::topology::configs::consensus::GeneralConsensusConfig,
    da_configs: &[testing_framework_config::topology::configs::da::GeneralDaConfig],
    blend_configs: &[testing_framework_config::topology::configs::blend::GeneralBlendConfig],
) -> Result<Vec<ProviderInfo>, TopologyBuildError> {
    let mut providers = Vec::with_capacity(da_configs.len() + blend_configs.len());

    for (i, da_conf) in da_configs.iter().enumerate() {
        let note = get_cloned("da_notes", &first_consensus.da_notes, i, da_configs.len())?;
        providers.push(ProviderInfo {
            service_type: ServiceType::DataAvailability,
            provider_sk: da_conf.signer.clone(),
            zk_sk: da_conf.secret_zk_key.clone(),
            locator: Locator(da_conf.listening_address.clone()),
            note,
        });
    }

    for (i, blend_conf) in blend_configs.iter().enumerate() {
        let note = get_cloned(
            "blend_notes",
            &first_consensus.blend_notes,
            i,
            blend_configs.len(),
        )?;
        providers.push(ProviderInfo {
            service_type: ServiceType::BlendNetwork,
            provider_sk: blend_conf.signer.clone(),
            zk_sk: blend_conf.secret_zk_key.clone(),
            locator: Locator(blend_conf.backend_core.listening_address.clone()),
            note,
        });
    }

    Ok(providers)
}

fn create_consensus_genesis_tx(
    first_consensus: &testing_framework_config::topology::configs::consensus::GeneralConsensusConfig,
    providers: Vec<ProviderInfo>,
) -> Result<nomos_core::mantle::genesis_tx::GenesisTx, TopologyBuildError> {
    let ledger_tx = first_consensus.genesis_tx.mantle_tx().ledger_tx.clone();
    Ok(create_genesis_tx_with_declarations(ledger_tx, providers)?)
}

fn apply_consensus_genesis_tx(
    consensus_configs: &mut [testing_framework_config::topology::configs::consensus::GeneralConsensusConfig],
    genesis_tx: &nomos_core::mantle::genesis_tx::GenesisTx,
) {
    for c in consensus_configs {
        c.genesis_tx = genesis_tx.clone();
    }
}

#[allow(clippy::too_many_arguments)]
fn build_node_descriptors(
    config: &TopologyConfig,
    n_participants: usize,
    ids: &[[u8; 32]],
    da_ports: &[u16],
    blend_ports: &[u16],
    consensus_configs: &[testing_framework_config::topology::configs::consensus::GeneralConsensusConfig],
    bootstrapping_config: &[testing_framework_config::topology::configs::bootstrap::GeneralBootstrapConfig],
    da_configs: &[testing_framework_config::topology::configs::da::GeneralDaConfig],
    network_configs: &[testing_framework_config::topology::configs::network::GeneralNetworkConfig],
    blend_configs: &[testing_framework_config::topology::configs::blend::GeneralBlendConfig],
    api_configs: &[testing_framework_config::topology::configs::api::GeneralApiConfig],
    tracing_configs: &[testing_framework_config::topology::configs::tracing::GeneralTracingConfig],
    kms_configs: &[key_management_system_service::backend::preload::PreloadKMSBackendSettings],
    time_config: &testing_framework_config::topology::configs::time::GeneralTimeConfig,
) -> Result<Vec<GeneratedNodeConfig>, TopologyBuildError> {
    let mut nodes = Vec::with_capacity(config.n_nodes);

    for i in 0..n_participants {
        let consensus_config =
            get_cloned("consensus_configs", consensus_configs, i, n_participants)?;
        let bootstrapping_config =
            get_cloned("bootstrap_configs", bootstrapping_config, i, n_participants)?;
        let da_config = get_cloned("da_configs", da_configs, i, n_participants)?;
        let network_config = get_cloned("network_configs", network_configs, i, n_participants)?;
        let blend_config = get_cloned("blend_configs", blend_configs, i, n_participants)?;
        let api_config = get_cloned("api_configs", api_configs, i, n_participants)?;
        let tracing_config = get_cloned("tracing_configs", tracing_configs, i, n_participants)?;
        let kms_config = get_cloned("kms_configs", kms_configs, i, n_participants)?;

        let id = get_copied("ids", ids, i, n_participants)?;
        let da_port = get_copied("da_ports", da_ports, i, n_participants)?;
        let blend_port = get_copied("blend_ports", blend_ports, i, n_participants)?;

        let general = GeneralConfig {
            consensus_config,
            bootstrapping_config,
            da_config,
            network_config,
            blend_config,
            api_config,
            tracing_config,
            time_config: time_config.clone(),
            kms_config,
        };

        let descriptor = GeneratedNodeConfig {
            index: i,
            id,
            general,
            da_port,
            blend_port,
        };
        nodes.push(descriptor);
    }

    Ok(nodes)
}

fn get_cloned<T: Clone>(
    label: &'static str,
    items: &[T],
    index: usize,
    expected: usize,
) -> Result<T, TopologyBuildError> {
    items
        .get(index)
        .cloned()
        .ok_or(TopologyBuildError::VectorLenMismatch {
            label,
            expected,
            actual: items.len(),
        })
}

fn get_copied<T: Copy>(
    label: &'static str,
    items: &[T],
    index: usize,
    expected: usize,
) -> Result<T, TopologyBuildError> {
    items
        .get(index)
        .copied()
        .ok_or(TopologyBuildError::VectorLenMismatch {
            label,
            expected,
            actual: items.len(),
        })
}
