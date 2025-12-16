use std::{
    num::{NonZeroU64, NonZeroUsize},
    time::Duration,
};

use testing_framework_core::{
    scenario::{Builder as CoreScenarioBuilder, NodeControlCapability, ObservabilityCapability},
    topology::configs::wallet::WalletConfig,
};

use crate::{
    expectations::ConsensusLiveness,
    workloads::{chaos::RandomRestartWorkload, da, transaction},
};

macro_rules! non_zero_rate_fn {
    ($name:ident, $message:literal) => {
        const fn $name(rate: u64) -> NonZeroU64 {
            match NonZeroU64::new(rate) {
                Some(value) => value,
                None => panic!($message),
            }
        }
    };
}

non_zero_rate_fn!(
    transaction_rate_checked,
    "transaction rate must be non-zero"
);

non_zero_rate_fn!(channel_rate_checked, "channel rate must be non-zero");
non_zero_rate_fn!(blob_rate_checked, "blob rate must be non-zero");

/// Extension methods for building test scenarios with common patterns.
pub trait ScenarioBuilderExt<Caps>: Sized {
    /// Configure a transaction flow workload.
    fn transactions(self) -> TransactionFlowBuilder<Caps>;

    /// Configure a transaction flow workload via closure.
    fn transactions_with(
        self,
        f: impl FnOnce(TransactionFlowBuilder<Caps>) -> TransactionFlowBuilder<Caps>,
    ) -> CoreScenarioBuilder<Caps>;

    /// Configure a data-availability workload.
    fn da(self) -> DataAvailabilityFlowBuilder<Caps>;

    /// Configure a data-availability workload via closure.
    fn da_with(
        self,
        f: impl FnOnce(DataAvailabilityFlowBuilder<Caps>) -> DataAvailabilityFlowBuilder<Caps>,
    ) -> CoreScenarioBuilder<Caps>;
    #[must_use]
    /// Attach a consensus liveness expectation.
    fn expect_consensus_liveness(self) -> Self;

    #[must_use]
    /// Seed deterministic wallets with total funds split across `users`.
    fn initialize_wallet(self, total_funds: u64, users: usize) -> Self;
}

impl<Caps> ScenarioBuilderExt<Caps> for CoreScenarioBuilder<Caps> {
    fn transactions(self) -> TransactionFlowBuilder<Caps> {
        TransactionFlowBuilder::new(self)
    }

    fn transactions_with(
        self,
        f: impl FnOnce(TransactionFlowBuilder<Caps>) -> TransactionFlowBuilder<Caps>,
    ) -> CoreScenarioBuilder<Caps> {
        f(self.transactions()).apply()
    }

    fn da(self) -> DataAvailabilityFlowBuilder<Caps> {
        DataAvailabilityFlowBuilder::new(self)
    }

    fn da_with(
        self,
        f: impl FnOnce(DataAvailabilityFlowBuilder<Caps>) -> DataAvailabilityFlowBuilder<Caps>,
    ) -> CoreScenarioBuilder<Caps> {
        f(self.da()).apply()
    }

    fn expect_consensus_liveness(self) -> Self {
        self.with_expectation(ConsensusLiveness::default())
    }

    fn initialize_wallet(self, total_funds: u64, users: usize) -> Self {
        let user_count = NonZeroUsize::new(users).expect("wallet user count must be non-zero");
        let wallet = WalletConfig::uniform(total_funds, user_count);
        self.with_wallet_config(wallet)
    }
}

/// Observability helpers for scenarios that want to reuse external telemetry.
pub trait ObservabilityBuilderExt: Sized {
    /// Reuse an existing Prometheus endpoint instead of provisioning one (k8s
    /// runner).
    fn with_external_prometheus(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability>;

    /// Convenience wrapper that parses a URL string (panics if invalid).
    fn with_external_prometheus_str(
        self,
        url: &str,
    ) -> CoreScenarioBuilder<ObservabilityCapability>;
}

impl ObservabilityBuilderExt for CoreScenarioBuilder<()> {
    fn with_external_prometheus(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_capabilities(ObservabilityCapability {
            external_prometheus: Some(url),
        })
    }

    fn with_external_prometheus_str(
        self,
        url: &str,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        let parsed = reqwest::Url::parse(url).expect("external prometheus url must be valid");
        self.with_external_prometheus(parsed)
    }
}

/// Builder for transaction workloads.
pub struct TransactionFlowBuilder<Caps> {
    builder: CoreScenarioBuilder<Caps>,
    rate: NonZeroU64,
    users: Option<NonZeroUsize>,
}

impl<Caps> TransactionFlowBuilder<Caps> {
    const fn default_rate() -> NonZeroU64 {
        transaction_rate_checked(1)
    }

    const fn new(builder: CoreScenarioBuilder<Caps>) -> Self {
        Self {
            builder,
            rate: Self::default_rate(),
            users: None,
        }
    }

    #[must_use]
    /// Set transaction submission rate per block (panics on zero).
    pub const fn rate(mut self, rate: u64) -> Self {
        self.rate = transaction_rate_checked(rate);
        self
    }

    #[must_use]
    /// Set transaction submission rate per block.
    pub const fn rate_per_block(mut self, rate: NonZeroU64) -> Self {
        self.rate = rate;
        self
    }

    #[must_use]
    /// Limit how many users will submit transactions.
    pub const fn users(mut self, users: usize) -> Self {
        match NonZeroUsize::new(users) {
            Some(value) => self.users = Some(value),
            None => panic!("transaction user count must be non-zero"),
        }
        self
    }

    #[must_use]
    /// Attach the transaction workload to the scenario.
    pub fn apply(mut self) -> CoreScenarioBuilder<Caps> {
        let workload = transaction::Workload::with_rate(self.rate.get())
            .expect("transaction rate must be non-zero")
            .with_user_limit(self.users);
        tracing::info!(
            rate = self.rate.get(),
            users = self.users.map(|u| u.get()),
            "attaching transaction workload"
        );

        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}

/// Builder for data availability workloads.
pub struct DataAvailabilityFlowBuilder<Caps> {
    builder: CoreScenarioBuilder<Caps>,
    channel_rate: NonZeroU64,
    blob_rate: NonZeroU64,
    headroom_percent: u64,
}

impl<Caps> DataAvailabilityFlowBuilder<Caps> {
    const fn default_channel_rate() -> NonZeroU64 {
        channel_rate_checked(1)
    }

    const fn default_blob_rate() -> NonZeroU64 {
        blob_rate_checked(1)
    }

    const fn new(builder: CoreScenarioBuilder<Caps>) -> Self {
        Self {
            builder,
            channel_rate: Self::default_channel_rate(),
            blob_rate: Self::default_blob_rate(),
            headroom_percent: da::Workload::default_headroom_percent(),
        }
    }

    #[must_use]
    /// Set the number of DA channels to run (panics on zero).
    pub const fn channel_rate(mut self, rate: u64) -> Self {
        self.channel_rate = channel_rate_checked(rate);
        self
    }

    #[must_use]
    /// Set the number of DA channels to run.
    pub const fn channel_rate_per_block(mut self, rate: NonZeroU64) -> Self {
        self.channel_rate = rate;
        self
    }

    #[must_use]
    /// Set blob publish rate (per block).
    pub const fn blob_rate(mut self, rate: u64) -> Self {
        self.blob_rate = blob_rate_checked(rate);
        self
    }

    #[must_use]
    /// Set blob publish rate per block.
    pub const fn blob_rate_per_block(mut self, rate: NonZeroU64) -> Self {
        self.blob_rate = rate;
        self
    }

    #[must_use]
    /// Apply headroom when converting blob rate into channel count.
    pub const fn headroom_percent(mut self, percent: u64) -> Self {
        self.headroom_percent = percent;
        self
    }

    #[must_use]
    pub fn apply(mut self) -> CoreScenarioBuilder<Caps> {
        let workload =
            da::Workload::with_rate(self.blob_rate, self.channel_rate, self.headroom_percent);
        tracing::info!(
            channel_rate = self.channel_rate.get(),
            blob_rate = self.blob_rate.get(),
            headroom_percent = self.headroom_percent,
            "attaching data-availability workload"
        );

        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}

/// Chaos helpers for scenarios that can control nodes.
pub trait ChaosBuilderExt: Sized {
    /// Entry point into chaos workloads.
    fn chaos(self) -> ChaosBuilder;

    /// Configure chaos via closure.
    fn chaos_with(
        self,
        f: impl FnOnce(ChaosBuilder) -> CoreScenarioBuilder<NodeControlCapability>,
    ) -> CoreScenarioBuilder<NodeControlCapability>;
}

impl ChaosBuilderExt for CoreScenarioBuilder<NodeControlCapability> {
    fn chaos(self) -> ChaosBuilder {
        ChaosBuilder { builder: self }
    }

    fn chaos_with(
        self,
        f: impl FnOnce(ChaosBuilder) -> CoreScenarioBuilder<NodeControlCapability>,
    ) -> CoreScenarioBuilder<NodeControlCapability> {
        f(self.chaos())
    }
}

/// Chaos workload builder root.
///
/// Start with `chaos()` on a scenario builder, then select a workload variant
/// such as `restart()`.
pub struct ChaosBuilder {
    builder: CoreScenarioBuilder<NodeControlCapability>,
}

impl ChaosBuilder {
    /// Finish without adding a chaos workload.
    #[must_use]
    pub fn apply(self) -> CoreScenarioBuilder<NodeControlCapability> {
        self.builder
    }

    /// Configure a random restarts chaos workload.
    #[must_use]
    pub fn restart(self) -> ChaosRestartBuilder {
        const DEFAULT_CHAOS_MIN_DELAY: Duration = Duration::from_secs(10);
        const DEFAULT_CHAOS_MAX_DELAY: Duration = Duration::from_secs(30);
        const DEFAULT_CHAOS_TARGET_COOLDOWN: Duration = Duration::from_secs(60);

        ChaosRestartBuilder {
            builder: self.builder,
            min_delay: DEFAULT_CHAOS_MIN_DELAY,
            max_delay: DEFAULT_CHAOS_MAX_DELAY,
            target_cooldown: DEFAULT_CHAOS_TARGET_COOLDOWN,
            include_validators: true,
            include_executors: true,
        }
    }
}

pub struct ChaosRestartBuilder {
    builder: CoreScenarioBuilder<NodeControlCapability>,
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
    include_validators: bool,
    include_executors: bool,
}

impl ChaosRestartBuilder {
    #[must_use]
    /// Set the minimum delay between restart operations.
    pub fn min_delay(mut self, delay: Duration) -> Self {
        assert!(!delay.is_zero(), "chaos restart min delay must be non-zero");
        self.min_delay = delay;
        self
    }

    #[must_use]
    /// Set the maximum delay between restart operations.
    pub fn max_delay(mut self, delay: Duration) -> Self {
        assert!(!delay.is_zero(), "chaos restart max delay must be non-zero");
        self.max_delay = delay;
        self
    }

    #[must_use]
    /// Cooldown to allow between restarts for a target node.
    pub fn target_cooldown(mut self, cooldown: Duration) -> Self {
        assert!(
            !cooldown.is_zero(),
            "chaos restart target cooldown must be non-zero"
        );
        self.target_cooldown = cooldown;
        self
    }

    #[must_use]
    /// Include validators in the restart target set.
    pub const fn include_validators(mut self, enabled: bool) -> Self {
        self.include_validators = enabled;
        self
    }

    #[must_use]
    /// Include executors in the restart target set.
    pub const fn include_executors(mut self, enabled: bool) -> Self {
        self.include_executors = enabled;
        self
    }

    #[must_use]
    /// Finalize the chaos restart workload and attach it to the scenario.
    pub fn apply(mut self) -> CoreScenarioBuilder<NodeControlCapability> {
        assert!(
            self.min_delay <= self.max_delay,
            "chaos restart min delay must not exceed max delay"
        );
        assert!(
            self.target_cooldown >= self.min_delay,
            "chaos restart target cooldown must be >= min delay"
        );
        assert!(
            self.include_validators || self.include_executors,
            "chaos restart requires at least one node group"
        );

        let workload = RandomRestartWorkload::new(
            self.min_delay,
            self.max_delay,
            self.target_cooldown,
            self.include_validators,
            self.include_executors,
        );
        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}
