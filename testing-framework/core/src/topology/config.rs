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
        network::{Libp2pNetworkLayout, NetworkParams},
        tracing::create_tracing_configs,
        wallet::WalletConfig,
    },
    invariants::TopologyInvariantError,
};
use thiserror::Error;

use crate::topology::{
    configs::{GeneralConfig, time::default_time_config},
    generation::{GeneratedNodeConfig, GeneratedTopology, NodeRole},
    utils::{TopologyResolveError, create_kms_configs, resolve_ids, resolve_ports},
};

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
    pub n_validators: usize,
    pub consensus_params: ConsensusParams,
    pub network_params: NetworkParams,
    pub wallet_config: WalletConfig,
}

impl TopologyConfig {
    /// Create a config with zero nodes; counts must be set before building.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            n_validators: 0,
            consensus_params: ConsensusParams::default_for_participants(1),
            network_params: NetworkParams::default(),
            wallet_config: WalletConfig::default(),
        }
    }

    #[must_use]
    /// Convenience config with two validators for consensus-only scenarios.
    pub fn two_validators() -> Self {
        Self {
            n_validators: 2,
            consensus_params: ConsensusParams::default_for_participants(2),
            network_params: NetworkParams::default(),
            wallet_config: WalletConfig::default(),
        }
    }

    #[must_use]
    /// Build a topology with explicit validator counts.
    pub fn with_node_numbers(validators: usize) -> Self {
        let participants = validators;

        Self {
            n_validators: validators,
            consensus_params: ConsensusParams::default_for_participants(participants),
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
    blend_ports: Option<Vec<u16>>,
}

impl TopologyBuilder {
    #[must_use]
    /// Create a builder from a base topology config.
    pub const fn new(config: TopologyConfig) -> Self {
        Self {
            config,
            ids: None,
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
    /// Override blend ports for nodes in order.
    pub fn with_blend_ports(mut self, ports: Vec<u16>) -> Self {
        self.blend_ports = Some(ports);
        self
    }

    #[must_use]
    pub const fn with_validator_count(mut self, validators: usize) -> Self {
        self.config.n_validators = validators;
        self
    }

    #[must_use]
    /// Set validator counts.
    pub const fn with_node_counts(mut self, validators: usize) -> Self {
        self.config.n_validators = validators;
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
            blend_ports,
        } = self;

        let n_participants = participant_count(&config)?;

        let (ids, blend_ports) = resolve_and_validate_vectors(ids, blend_ports, n_participants)?;

        let BaseConfigs {
            mut consensus_configs,
            bootstrap_configs: bootstrapping_config,
            network_configs,
            blend_configs,
        } = build_base_configs(
            &ids,
            &config.consensus_params,
            &config.network_params,
            &config.wallet_config,
            &blend_ports,
        )?;

        let api_configs = create_api_configs(&ids)?;
        let tracing_configs = create_tracing_configs(&ids);
        let time_config = default_time_config();

        let first_consensus = consensus_configs
            .first()
            .ok_or(TopologyBuildError::MissingConsensusConfig)?;
        let providers = collect_provider_infos(first_consensus, &blend_configs)?;

        let genesis_tx = create_consensus_genesis_tx(first_consensus, providers)?;
        apply_consensus_genesis_tx(&mut consensus_configs, &genesis_tx);

        let kms_configs = create_kms_configs(&blend_configs, &config.wallet_config.accounts);

        let validators = build_node_descriptors(
            &config,
            n_participants,
            &ids,
            &blend_ports,
            &consensus_configs,
            &bootstrapping_config,
            &network_configs,
            &blend_configs,
            &api_configs,
            &tracing_configs,
            &kms_configs,
            &time_config,
        )?;

        Ok(GeneratedTopology { config, validators })
    }

    #[must_use]
    pub const fn config(&self) -> &TopologyConfig {
        &self.config
    }
}

fn participant_count(config: &TopologyConfig) -> Result<usize, TopologyBuildError> {
    let n_participants = config.n_validators;
    if n_participants == 0 {
        return Err(TopologyBuildError::EmptyParticipants);
    }

    Ok(n_participants)
}

fn resolve_and_validate_vectors(
    ids: Option<Vec<[u8; 32]>>,
    blend_ports: Option<Vec<u16>>,
    n_participants: usize,
) -> Result<(Vec<[u8; 32]>, Vec<u16>), TopologyBuildError> {
    let ids = resolve_ids(ids, n_participants)?;
    let blend_ports = resolve_ports(blend_ports, n_participants, "Blend")?;

    Ok((ids, blend_ports))
}

fn collect_provider_infos(
    first_consensus: &testing_framework_config::topology::configs::consensus::GeneralConsensusConfig,
    blend_configs: &[testing_framework_config::topology::configs::blend::GeneralBlendConfig],
) -> Result<Vec<ProviderInfo>, TopologyBuildError> {
    let mut providers = Vec::with_capacity(blend_configs.len());

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
    blend_ports: &[u16],
    consensus_configs: &[testing_framework_config::topology::configs::consensus::GeneralConsensusConfig],
    bootstrapping_config: &[testing_framework_config::topology::configs::bootstrap::GeneralBootstrapConfig],
    network_configs: &[testing_framework_config::topology::configs::network::GeneralNetworkConfig],
    blend_configs: &[testing_framework_config::topology::configs::blend::GeneralBlendConfig],
    api_configs: &[testing_framework_config::topology::configs::api::GeneralApiConfig],
    tracing_configs: &[testing_framework_config::topology::configs::tracing::GeneralTracingConfig],
    kms_configs: &[key_management_system_service::backend::preload::PreloadKMSBackendSettings],
    time_config: &testing_framework_config::topology::configs::time::GeneralTimeConfig,
) -> Result<Vec<GeneratedNodeConfig>, TopologyBuildError> {
    let mut validators = Vec::with_capacity(config.n_validators);

    for i in 0..n_participants {
        let consensus_config =
            get_cloned("consensus_configs", consensus_configs, i, n_participants)?;
        let bootstrapping_config =
            get_cloned("bootstrap_configs", bootstrapping_config, i, n_participants)?;
        let network_config = get_cloned("network_configs", network_configs, i, n_participants)?;
        let blend_config = get_cloned("blend_configs", blend_configs, i, n_participants)?;
        let api_config = get_cloned("api_configs", api_configs, i, n_participants)?;
        let tracing_config = get_cloned("tracing_configs", tracing_configs, i, n_participants)?;
        let kms_config = get_cloned("kms_configs", kms_configs, i, n_participants)?;

        let id = get_copied("ids", ids, i, n_participants)?;
        let blend_port = get_copied("blend_ports", blend_ports, i, n_participants)?;

        let general = GeneralConfig {
            consensus_config,
            bootstrapping_config,
            network_config,
            blend_config,
            api_config,
            tracing_config,
            time_config: time_config.clone(),
            kms_config,
        };

        let (role, index) = (NodeRole::Validator, i);
        let descriptor = GeneratedNodeConfig {
            role,
            index,
            id,
            general,
            blend_port,
        };

        match role {
            NodeRole::Validator => validators.push(descriptor),
        }
    }

    Ok(validators)
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
