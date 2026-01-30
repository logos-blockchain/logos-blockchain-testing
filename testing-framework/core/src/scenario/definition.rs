use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use nomos_node::config::RunConfig;
use thiserror::Error;
use tracing::{debug, info};

use super::{
    DynError, NodeControlCapability, expectation::Expectation, runtime::context::RunMetrics,
    workload::Workload,
};
use crate::topology::{
    config::{NodeConfigPatch, TopologyBuildError, TopologyBuilder, TopologyConfig},
    configs::{network::Libp2pNetworkLayout, wallet::WalletConfig},
    generation::GeneratedTopology,
};

const DEFAULT_FUNDS_PER_WALLET: u64 = 100;
const MIN_EXPECTATION_BLOCKS: u32 = 2;
const MIN_EXPECTATION_FALLBACK_SECS: u64 = 10;

#[derive(Debug, Error)]
pub enum ScenarioBuildError {
    #[error(transparent)]
    Topology(#[from] TopologyBuildError),
    #[error("wallet user count must be non-zero (got {users})")]
    WalletUsersZero { users: usize },
    #[error("wallet funds overflow for {users} users at {per_wallet} per wallet")]
    WalletFundsOverflow { users: usize, per_wallet: u64 },
    #[error("workload '{name}' failed to initialize")]
    WorkloadInit { name: String, source: DynError },
    #[error("expectation '{name}' failed to initialize")]
    ExpectationInit { name: String, source: DynError },
}

/// Immutable scenario definition shared between the runner, workloads, and
/// expectations.
pub struct Scenario<Caps = ()> {
    topology: GeneratedTopology,
    workloads: Vec<Arc<dyn Workload>>,
    expectations: Vec<Box<dyn Expectation>>,
    duration: Duration,
    capabilities: Caps,
}

impl<Caps> Scenario<Caps> {
    fn new(
        topology: GeneratedTopology,
        workloads: Vec<Arc<dyn Workload>>,
        expectations: Vec<Box<dyn Expectation>>,
        duration: Duration,
        capabilities: Caps,
    ) -> Self {
        Self {
            topology,
            workloads,
            expectations,
            duration,
            capabilities,
        }
    }

    #[must_use]
    pub const fn topology(&self) -> &GeneratedTopology {
        &self.topology
    }

    #[must_use]
    pub fn workloads(&self) -> &[Arc<dyn Workload>] {
        &self.workloads
    }

    #[must_use]
    pub fn expectations(&self) -> &[Box<dyn Expectation>] {
        &self.expectations
    }

    #[must_use]
    pub fn expectations_mut(&mut self) -> &mut [Box<dyn Expectation>] {
        &mut self.expectations
    }

    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    #[must_use]
    pub const fn capabilities(&self) -> &Caps {
        &self.capabilities
    }
}

/// Builder used by callers to describe the desired scenario.
pub struct Builder<Caps = ()> {
    topology: TopologyBuilder,
    workloads: Vec<Box<dyn Workload>>,
    expectations: Vec<Box<dyn Expectation>>,
    duration: Duration,
    wallet_users: Option<usize>,
    capabilities: Caps,
}

pub type ScenarioBuilder = Builder<()>;

/// Builder for shaping the scenario topology.
pub struct TopologyConfigurator<Caps> {
    builder: Builder<Caps>,
    nodes: usize,
    network_star: bool,
}

impl<Caps: Default> Builder<Caps> {
    #[must_use]
    /// Start a builder from a topology description.
    pub fn new(topology: TopologyBuilder) -> Self {
        Self {
            topology,
            workloads: Vec::new(),
            expectations: Vec::new(),
            duration: Duration::ZERO,
            wallet_users: None,
            capabilities: Caps::default(),
        }
    }

    #[must_use]
    pub fn with_node_counts(nodes: usize) -> Self {
        Self::new(TopologyBuilder::new(TopologyConfig::with_node_numbers(
            nodes,
        )))
    }

    /// Convenience constructor that immediately enters topology configuration,
    /// letting callers set counts via `nodes`.
    pub fn topology() -> TopologyConfigurator<Caps> {
        TopologyConfigurator::new(Self::new(TopologyBuilder::new(TopologyConfig::empty())))
    }

    /// Configure topology via a closure and return the scenario builder.
    #[must_use]
    pub fn topology_with(
        f: impl FnOnce(TopologyConfigurator<Caps>) -> TopologyConfigurator<Caps>,
    ) -> Builder<Caps> {
        let configurator = Self::topology();
        f(configurator).apply()
    }
}

impl<Caps> Builder<Caps> {
    #[must_use]
    /// Swap capabilities type carried with the scenario.
    pub fn with_capabilities<NewCaps>(self, capabilities: NewCaps) -> Builder<NewCaps> {
        let Self {
            topology,
            workloads,
            expectations,
            duration,
            wallet_users,
            ..
        } = self;

        Builder {
            topology,
            workloads,
            expectations,
            duration,
            wallet_users,
            capabilities,
        }
    }

    #[must_use]
    pub const fn capabilities(&self) -> &Caps {
        &self.capabilities
    }

    #[must_use]
    pub const fn capabilities_mut(&mut self) -> &mut Caps {
        &mut self.capabilities
    }

    #[must_use]
    pub fn with_workload<W>(mut self, workload: W) -> Self
    where
        W: Workload + 'static,
    {
        self.expectations.extend(workload.expectations());
        self.workloads.push(Box::new(workload));
        self
    }

    #[must_use]
    /// Add a standalone expectation not tied to a workload.
    pub fn with_expectation<E>(mut self, expectation: E) -> Self
    where
        E: Expectation + 'static,
    {
        self.expectations.push(Box::new(expectation));
        self
    }

    #[must_use]
    /// Configure the intended run duration.
    pub const fn with_run_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    #[must_use]
    /// Transform the topology builder.
    pub fn map_topology(mut self, f: impl FnOnce(TopologyBuilder) -> TopologyBuilder) -> Self {
        self.topology = f(self.topology);
        self
    }

    #[must_use]
    /// Override wallet config for the topology.
    pub fn with_wallet_config(mut self, wallet: WalletConfig) -> Self {
        self.topology = self.topology.with_wallet_config(wallet);
        self.wallet_users = None;
        self
    }

    #[must_use]
    pub fn wallets(self, users: usize) -> Self {
        let mut builder = self;
        builder.wallet_users = Some(users);
        builder
    }

    #[must_use]
    /// Finalize the scenario, computing run metrics and initializing
    /// components.
    pub fn build(self) -> Result<Scenario<Caps>, ScenarioBuildError> {
        let Self {
            mut topology,
            mut workloads,
            mut expectations,
            duration,
            wallet_users,
            capabilities,
            ..
        } = self;

        if let Some(users) = wallet_users {
            let user_count =
                NonZeroUsize::new(users).ok_or(ScenarioBuildError::WalletUsersZero { users })?;
            let total_funds = DEFAULT_FUNDS_PER_WALLET.checked_mul(users as u64).ok_or(
                ScenarioBuildError::WalletFundsOverflow {
                    users,
                    per_wallet: DEFAULT_FUNDS_PER_WALLET,
                },
            )?;

            let wallet = WalletConfig::uniform(total_funds, user_count);
            topology = topology.with_wallet_config(wallet);
        }

        let generated = topology.build()?;
        let duration = enforce_min_duration(&generated, duration);
        let run_metrics = RunMetrics::from_topology(&generated, duration);
        initialize_components(&generated, &run_metrics, &mut workloads, &mut expectations)?;
        let workloads: Vec<Arc<dyn Workload>> = workloads.into_iter().map(Arc::from).collect();

        info!(
            nodes = generated.nodes().len(),
            duration_secs = duration.as_secs(),
            workloads = workloads.len(),
            expectations = expectations.len(),
            "scenario built"
        );

        Ok(Scenario::new(
            generated,
            workloads,
            expectations,
            duration,
            capabilities,
        ))
    }
}

impl<Caps> TopologyConfigurator<Caps> {
    const fn new(builder: Builder<Caps>) -> Self {
        Self {
            builder,
            nodes: 0,
            network_star: false,
        }
    }

    /// Set the number of nodes.
    #[must_use]
    pub fn nodes(mut self, count: usize) -> Self {
        self.nodes = count;
        self
    }

    /// Use a star libp2p network layout.
    #[must_use]
    pub fn network_star(mut self) -> Self {
        self.network_star = true;
        self
    }

    /// Apply a config patch for a specific node index.
    #[must_use]
    pub fn node_config_patch(mut self, index: usize, patch: NodeConfigPatch) -> Self {
        self.builder.topology = self.builder.topology.with_node_config_patch(index, patch);
        self
    }

    /// Apply a config patch for a specific node index.
    #[must_use]
    pub fn node_config_patch_with<F>(mut self, index: usize, f: F) -> Self
    where
        F: Fn(RunConfig) -> Result<RunConfig, DynError> + Send + Sync + 'static,
    {
        self.builder.topology = self
            .builder
            .topology
            .with_node_config_patch(index, Arc::new(f));
        self
    }

    /// Finalize and return the underlying scenario builder.
    #[must_use]
    pub fn apply(self) -> Builder<Caps> {
        let mut builder = self.builder;
        builder.topology = builder.topology.with_node_count(self.nodes);

        if self.network_star {
            builder.topology = builder
                .topology
                .with_network_layout(Libp2pNetworkLayout::Star);
        }
        builder
    }
}

impl Builder<()> {
    #[must_use]
    pub fn enable_node_control(self) -> Builder<NodeControlCapability> {
        self.with_capabilities(NodeControlCapability)
    }
}

fn initialize_components(
    descriptors: &GeneratedTopology,
    run_metrics: &RunMetrics,
    workloads: &mut [Box<dyn Workload>],
    expectations: &mut [Box<dyn Expectation>],
) -> Result<(), ScenarioBuildError> {
    initialize_workloads(descriptors, run_metrics, workloads)?;
    initialize_expectations(descriptors, run_metrics, expectations)?;
    Ok(())
}

fn initialize_workloads(
    descriptors: &GeneratedTopology,
    run_metrics: &RunMetrics,
    workloads: &mut [Box<dyn Workload>],
) -> Result<(), ScenarioBuildError> {
    for workload in workloads {
        debug!(workload = workload.name(), "initializing workload");
        workload.init(descriptors, run_metrics).map_err(|source| {
            ScenarioBuildError::WorkloadInit {
                name: workload.name().to_owned(),
                source,
            }
        })?;
    }
    Ok(())
}

fn initialize_expectations(
    descriptors: &GeneratedTopology,
    run_metrics: &RunMetrics,
    expectations: &mut [Box<dyn Expectation>],
) -> Result<(), ScenarioBuildError> {
    for expectation in expectations {
        debug!(expectation = expectation.name(), "initializing expectation");
        expectation
            .init(descriptors, run_metrics)
            .map_err(|source| ScenarioBuildError::ExpectationInit {
                name: expectation.name().to_owned(),
                source,
            })?;
    }
    Ok(())
}

fn enforce_min_duration(descriptors: &GeneratedTopology, requested: Duration) -> Duration {
    let min_duration = descriptors.slot_duration().map_or_else(
        || Duration::from_secs(MIN_EXPECTATION_FALLBACK_SECS),
        |slot| slot * MIN_EXPECTATION_BLOCKS,
    );

    requested.max(min_duration)
}
