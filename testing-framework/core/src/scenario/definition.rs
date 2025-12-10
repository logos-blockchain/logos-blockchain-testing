use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use super::{
    NodeControlCapability, expectation::Expectation, runtime::context::RunMetrics,
    workload::Workload,
};
use crate::topology::{
    config::{TopologyBuilder, TopologyConfig},
    configs::{network::Libp2pNetworkLayout, wallet::WalletConfig},
    generation::GeneratedTopology,
};

const DEFAULT_FUNDS_PER_WALLET: u64 = 100;
const MIN_EXPECTATION_BLOCKS: u32 = 2;
const MIN_EXPECTATION_FALLBACK_SECS: u64 = 10;

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
    workloads: Vec<Arc<dyn Workload>>,
    expectations: Vec<Box<dyn Expectation>>,
    duration: Duration,
    capabilities: Caps,
}

pub type ScenarioBuilder = Builder<()>;

/// Builder for shaping the scenario topology.
pub struct TopologyConfigurator<Caps> {
    builder: Builder<Caps>,
    validators: usize,
    executors: usize,
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
            capabilities: Caps::default(),
        }
    }

    #[must_use]
    pub fn with_node_counts(validators: usize, executors: usize) -> Self {
        Self::new(TopologyBuilder::new(TopologyConfig::with_node_numbers(
            validators, executors,
        )))
    }

    /// Convenience constructor that immediately enters topology configuration,
    /// letting callers set counts via `validators`/`executors`.
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
            ..
        } = self;

        Builder {
            topology,
            workloads,
            expectations,
            duration,
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
        self.workloads.push(Arc::new(workload));
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
        self
    }

    #[must_use]
    pub fn wallets(self, users: usize) -> Self {
        let user_count = NonZeroUsize::new(users).expect("wallet user count must be non-zero");
        let total_funds = DEFAULT_FUNDS_PER_WALLET
            .checked_mul(users as u64)
            .expect("wallet count exceeds capacity");
        let wallet = WalletConfig::uniform(total_funds, user_count);
        self.with_wallet_config(wallet)
    }

    #[must_use]
    /// Finalize the scenario, computing run metrics and initializing
    /// components.
    pub fn build(self) -> Scenario<Caps> {
        let Self {
            topology,
            mut workloads,
            mut expectations,
            duration,
            capabilities,
            ..
        } = self;

        let generated = topology.build();
        let duration = enforce_min_duration(&generated, duration);
        let run_metrics = RunMetrics::from_topology(&generated, duration);
        initialize_components(&generated, &run_metrics, &mut workloads, &mut expectations);

        Scenario::new(generated, workloads, expectations, duration, capabilities)
    }
}

impl<Caps> TopologyConfigurator<Caps> {
    const fn new(builder: Builder<Caps>) -> Self {
        Self {
            builder,
            validators: 0,
            executors: 0,
            network_star: false,
        }
    }

    /// Set the number of validator nodes.
    #[must_use]
    pub fn validators(mut self, count: usize) -> Self {
        self.validators = count;
        self
    }

    /// Set the number of executor nodes.
    #[must_use]
    pub fn executors(mut self, count: usize) -> Self {
        self.executors = count;
        self
    }

    /// Use a star libp2p network layout.
    #[must_use]
    pub fn network_star(mut self) -> Self {
        self.network_star = true;
        self
    }

    /// Finalize and return the underlying scenario builder.
    #[must_use]
    pub fn apply(self) -> Builder<Caps> {
        let participants = self.validators + self.executors;
        assert!(
            participants > 0,
            "topology must include at least one node; call validators()/executors() before apply()"
        );

        let mut config = TopologyConfig::with_node_numbers(self.validators, self.executors);
        if self.network_star {
            config.network_params.libp2p_network_layout = Libp2pNetworkLayout::Star;
        }

        let mut builder = self.builder;
        builder.topology = TopologyBuilder::new(config);
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
    workloads: &mut [Arc<dyn Workload>],
    expectations: &mut [Box<dyn Expectation>],
) {
    initialize_workloads(descriptors, run_metrics, workloads);
    initialize_expectations(descriptors, run_metrics, expectations);
}

fn initialize_workloads(
    descriptors: &GeneratedTopology,
    run_metrics: &RunMetrics,
    workloads: &mut [Arc<dyn Workload>],
) {
    for workload in workloads {
        let inner =
            Arc::get_mut(workload).expect("workload unexpectedly cloned before initialization");
        if let Err(err) = inner.init(descriptors, run_metrics) {
            panic!("workload '{}' failed to initialize: {err}", inner.name());
        }
    }
}

fn initialize_expectations(
    descriptors: &GeneratedTopology,
    run_metrics: &RunMetrics,
    expectations: &mut [Box<dyn Expectation>],
) {
    for expectation in expectations {
        if let Err(err) = expectation.init(descriptors, run_metrics) {
            panic!(
                "expectation '{}' failed to initialize: {err}",
                expectation.name()
            );
        }
    }
}

fn enforce_min_duration(descriptors: &GeneratedTopology, requested: Duration) -> Duration {
    let min_duration = descriptors.slot_duration().map_or_else(
        || Duration::from_secs(MIN_EXPECTATION_FALLBACK_SECS),
        |slot| slot * MIN_EXPECTATION_BLOCKS,
    );

    requested.max(min_duration)
}
