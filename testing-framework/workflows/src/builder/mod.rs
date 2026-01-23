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
    workloads::{chaos::RandomRestartWorkload, transaction},
};

#[derive(Debug, thiserror::Error)]
pub enum BuilderInputError {
    #[error("{field} must be non-zero")]
    ZeroValue { field: &'static str },
    #[error("invalid url for {field}: '{value}': {message}")]
    InvalidUrl {
        field: &'static str,
        value: String,
        message: String,
    },
}

/// Extension methods for building test scenarios with common patterns.
pub trait ScenarioBuilderExt<Caps>: Sized {
    /// Configure a transaction flow workload.
    fn transactions(self) -> TransactionFlowBuilder<Caps>;

    /// Configure a transaction flow workload via closure.
    fn transactions_with(
        self,
        f: impl FnOnce(TransactionFlowBuilder<Caps>) -> TransactionFlowBuilder<Caps>,
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

    fn expect_consensus_liveness(self) -> Self {
        self.with_expectation(ConsensusLiveness::default())
    }

    fn initialize_wallet(self, total_funds: u64, users: usize) -> Self {
        let Some(user_count) = NonZeroUsize::new(users) else {
            tracing::warn!(
                users,
                "wallet user count must be non-zero; ignoring initialize_wallet"
            );
            return self;
        };
        self.with_wallet_config(WalletConfig::uniform(total_funds, user_count))
    }
}

/// Observability helpers for scenarios that want to reuse external telemetry.
pub trait ObservabilityBuilderExt: Sized {
    /// Reuse an existing Prometheus endpoint instead of provisioning one (k8s
    /// runner).
    fn with_metrics_query_url(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability>;

    /// Convenience wrapper that parses a URL string (panics if invalid).
    fn with_metrics_query_url_str(self, url: &str) -> CoreScenarioBuilder<ObservabilityCapability>;

    /// Like `with_metrics_query_url_str`, but returns an error instead of
    /// panicking.
    fn try_with_metrics_query_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError>;

    /// Configure the OTLP HTTP metrics ingest endpoint to which nodes should
    /// export metrics (must be a full URL, including any required path).
    fn with_metrics_otlp_ingest_url(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability>;

    /// Convenience wrapper that parses a URL string (panics if invalid).
    fn with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> CoreScenarioBuilder<ObservabilityCapability>;

    /// Like `with_metrics_otlp_ingest_url_str`, but returns an error instead of
    /// panicking.
    fn try_with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError>;

    /// Optional Grafana base URL for printing/logging (human access).
    fn with_grafana_url(self, url: reqwest::Url) -> CoreScenarioBuilder<ObservabilityCapability>;

    /// Convenience wrapper that parses a URL string (panics if invalid).
    fn with_grafana_url_str(self, url: &str) -> CoreScenarioBuilder<ObservabilityCapability>;

    /// Like `with_grafana_url_str`, but returns an error instead of panicking.
    fn try_with_grafana_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError>;

    #[deprecated(note = "use with_metrics_query_url")]
    fn with_external_prometheus(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_metrics_query_url(url)
    }

    #[deprecated(note = "use with_metrics_query_url_str")]
    fn with_external_prometheus_str(
        self,
        url: &str,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_metrics_query_url_str(url)
    }

    #[deprecated(note = "use with_metrics_otlp_ingest_url")]
    fn with_external_otlp_metrics_endpoint(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_metrics_otlp_ingest_url(url)
    }

    #[deprecated(note = "use with_metrics_otlp_ingest_url_str")]
    fn with_external_otlp_metrics_endpoint_str(
        self,
        url: &str,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_metrics_otlp_ingest_url_str(url)
    }
}

impl ObservabilityBuilderExt for CoreScenarioBuilder<()> {
    fn with_metrics_query_url(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_capabilities(ObservabilityCapability {
            metrics_query_url: Some(url),
            metrics_otlp_ingest_url: None,
            grafana_url: None,
        })
    }

    fn with_metrics_query_url_str(self, url: &str) -> CoreScenarioBuilder<ObservabilityCapability> {
        match reqwest::Url::parse(url) {
            Ok(parsed) => self.with_metrics_query_url(parsed),
            Err(err) => {
                tracing::warn!(
                    url,
                    error = %err,
                    "metrics query url must be valid; leaving metrics_query_url unset"
                );
                self.with_capabilities(ObservabilityCapability {
                    metrics_query_url: None,
                    metrics_otlp_ingest_url: None,
                    grafana_url: None,
                })
            }
        }
    }

    fn try_with_metrics_query_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError> {
        let parsed = reqwest::Url::parse(url).map_err(|err| BuilderInputError::InvalidUrl {
            field: "metrics_query_url",
            value: url.to_string(),
            message: err.to_string(),
        })?;
        Ok(self.with_metrics_query_url(parsed))
    }

    fn with_metrics_otlp_ingest_url(
        self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_capabilities(ObservabilityCapability {
            metrics_query_url: None,
            metrics_otlp_ingest_url: Some(url),
            grafana_url: None,
        })
    }

    fn with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        match reqwest::Url::parse(url) {
            Ok(parsed) => self.with_metrics_otlp_ingest_url(parsed),
            Err(err) => {
                tracing::warn!(
                    url,
                    error = %err,
                    "metrics OTLP ingest url must be valid; leaving metrics_otlp_ingest_url unset"
                );
                self.with_capabilities(ObservabilityCapability {
                    metrics_query_url: None,
                    metrics_otlp_ingest_url: None,
                    grafana_url: None,
                })
            }
        }
    }

    fn try_with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError> {
        let parsed = reqwest::Url::parse(url).map_err(|err| BuilderInputError::InvalidUrl {
            field: "metrics_otlp_ingest_url",
            value: url.to_string(),
            message: err.to_string(),
        })?;
        Ok(self.with_metrics_otlp_ingest_url(parsed))
    }

    fn with_grafana_url(self, url: reqwest::Url) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.with_capabilities(ObservabilityCapability {
            metrics_query_url: None,
            metrics_otlp_ingest_url: None,
            grafana_url: Some(url),
        })
    }

    fn with_grafana_url_str(self, url: &str) -> CoreScenarioBuilder<ObservabilityCapability> {
        match reqwest::Url::parse(url) {
            Ok(parsed) => self.with_grafana_url(parsed),
            Err(err) => {
                tracing::warn!(
                    url,
                    error = %err,
                    "grafana url must be valid; leaving grafana_url unset"
                );
                self.with_capabilities(ObservabilityCapability {
                    metrics_query_url: None,
                    metrics_otlp_ingest_url: None,
                    grafana_url: None,
                })
            }
        }
    }

    fn try_with_grafana_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError> {
        let parsed = reqwest::Url::parse(url).map_err(|err| BuilderInputError::InvalidUrl {
            field: "grafana_url",
            value: url.to_string(),
            message: err.to_string(),
        })?;
        Ok(self.with_grafana_url(parsed))
    }
}

impl ObservabilityBuilderExt for CoreScenarioBuilder<ObservabilityCapability> {
    fn with_metrics_query_url(
        mut self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.capabilities_mut().metrics_query_url = Some(url);
        self
    }

    fn with_metrics_query_url_str(self, url: &str) -> CoreScenarioBuilder<ObservabilityCapability> {
        match reqwest::Url::parse(url) {
            Ok(parsed) => self.with_metrics_query_url(parsed),
            Err(err) => {
                tracing::warn!(
                    url,
                    error = %err,
                    "metrics query url must be valid; leaving metrics_query_url unchanged"
                );
                self
            }
        }
    }

    fn try_with_metrics_query_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError> {
        let parsed = reqwest::Url::parse(url).map_err(|err| BuilderInputError::InvalidUrl {
            field: "metrics_query_url",
            value: url.to_string(),
            message: err.to_string(),
        })?;
        Ok(self.with_metrics_query_url(parsed))
    }

    fn with_metrics_otlp_ingest_url(
        mut self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.capabilities_mut().metrics_otlp_ingest_url = Some(url);
        self
    }

    fn with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        match reqwest::Url::parse(url) {
            Ok(parsed) => self.with_metrics_otlp_ingest_url(parsed),
            Err(err) => {
                tracing::warn!(
                    url,
                    error = %err,
                    "metrics OTLP ingest url must be valid; leaving metrics_otlp_ingest_url unchanged"
                );
                self
            }
        }
    }

    fn try_with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError> {
        let parsed = reqwest::Url::parse(url).map_err(|err| BuilderInputError::InvalidUrl {
            field: "metrics_otlp_ingest_url",
            value: url.to_string(),
            message: err.to_string(),
        })?;
        Ok(self.with_metrics_otlp_ingest_url(parsed))
    }

    fn with_grafana_url(
        mut self,
        url: reqwest::Url,
    ) -> CoreScenarioBuilder<ObservabilityCapability> {
        self.capabilities_mut().grafana_url = Some(url);
        self
    }

    fn with_grafana_url_str(self, url: &str) -> CoreScenarioBuilder<ObservabilityCapability> {
        match reqwest::Url::parse(url) {
            Ok(parsed) => self.with_grafana_url(parsed),
            Err(err) => {
                tracing::warn!(
                    url,
                    error = %err,
                    "grafana url must be valid; leaving grafana_url unchanged"
                );
                self
            }
        }
    }

    fn try_with_grafana_url_str(
        self,
        url: &str,
    ) -> Result<CoreScenarioBuilder<ObservabilityCapability>, BuilderInputError> {
        let parsed = reqwest::Url::parse(url).map_err(|err| BuilderInputError::InvalidUrl {
            field: "grafana_url",
            value: url.to_string(),
            message: err.to_string(),
        })?;
        Ok(self.with_grafana_url(parsed))
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
        NonZeroU64::MIN
    }

    const fn new(builder: CoreScenarioBuilder<Caps>) -> Self {
        Self {
            builder,
            rate: Self::default_rate(),
            users: None,
        }
    }

    #[must_use]
    /// Set transaction submission rate per block (ignores zero).
    pub fn rate(mut self, rate: u64) -> Self {
        match NonZeroU64::new(rate) {
            Some(rate) => self.rate = rate,
            None => tracing::warn!(
                rate,
                "transaction rate must be non-zero; keeping previous rate"
            ),
        }
        self
    }

    /// Like `rate`, but returns an error instead of panicking.
    pub fn try_rate(self, rate: u64) -> Result<Self, BuilderInputError> {
        let Some(rate) = NonZeroU64::new(rate) else {
            return Err(BuilderInputError::ZeroValue {
                field: "transaction_rate",
            });
        };
        Ok(self.rate_per_block(rate))
    }

    #[must_use]
    /// Set transaction submission rate per block.
    pub const fn rate_per_block(mut self, rate: NonZeroU64) -> Self {
        self.rate = rate;
        self
    }

    #[must_use]
    /// Limit how many users will submit transactions.
    pub fn users(mut self, users: usize) -> Self {
        match NonZeroUsize::new(users) {
            Some(value) => self.users = Some(value),
            None => tracing::warn!(
                users,
                "transaction user count must be non-zero; keeping previous setting"
            ),
        };
        self
    }

    /// Like `users`, but returns an error instead of panicking.
    pub fn try_users(mut self, users: usize) -> Result<Self, BuilderInputError> {
        let Some(value) = NonZeroUsize::new(users) else {
            return Err(BuilderInputError::ZeroValue {
                field: "transaction_users",
            });
        };
        self.users = Some(value);
        Ok(self)
    }

    #[must_use]
    /// Attach the transaction workload to the scenario.
    pub fn apply(mut self) -> CoreScenarioBuilder<Caps> {
        let workload = transaction::Workload::new(self.rate).with_user_limit(self.users);

        tracing::info!(
            rate = self.rate.get(),
            users = self.users.map(|u| u.get()),
            "attaching transaction workload"
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
        }
    }
}

pub struct ChaosRestartBuilder {
    builder: CoreScenarioBuilder<NodeControlCapability>,
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
    include_validators: bool,
}

impl ChaosRestartBuilder {
    #[must_use]
    /// Set the minimum delay between restart operations.
    pub fn min_delay(mut self, delay: Duration) -> Self {
        if delay.is_zero() {
            tracing::warn!("chaos restart min delay must be non-zero; keeping previous value");
        } else {
            self.min_delay = delay;
        }
        self
    }

    #[must_use]
    /// Set the maximum delay between restart operations.
    pub fn max_delay(mut self, delay: Duration) -> Self {
        if delay.is_zero() {
            tracing::warn!("chaos restart max delay must be non-zero; keeping previous value");
        } else {
            self.max_delay = delay;
        }
        self
    }

    #[must_use]
    /// Cooldown to allow between restarts for a target node.
    pub fn target_cooldown(mut self, cooldown: Duration) -> Self {
        if cooldown.is_zero() {
            tracing::warn!(
                "chaos restart target cooldown must be non-zero; keeping previous value"
            );
        } else {
            self.target_cooldown = cooldown;
        }
        self
    }

    #[must_use]
    /// Include validators in the restart target set.
    pub const fn include_validators(mut self, enabled: bool) -> Self {
        self.include_validators = enabled;
        self
    }

    #[must_use]
    /// Finalize the chaos restart workload and attach it to the scenario.
    pub fn apply(mut self) -> CoreScenarioBuilder<NodeControlCapability> {
        if self.min_delay > self.max_delay {
            tracing::warn!(
                min_delay_secs = self.min_delay.as_secs(),
                max_delay_secs = self.max_delay.as_secs(),
                "chaos restart min delay exceeds max delay; swapping"
            );
            std::mem::swap(&mut self.min_delay, &mut self.max_delay);
        }
        if self.target_cooldown < self.min_delay {
            tracing::warn!(
                target_cooldown_secs = self.target_cooldown.as_secs(),
                min_delay_secs = self.min_delay.as_secs(),
                "chaos restart target cooldown must be >= min delay; bumping cooldown"
            );
            self.target_cooldown = self.min_delay;
        }
        if !self.include_validators {
            tracing::warn!("chaos restart requires at least one node group; enabling all targets");
            self.include_validators = true;
        }

        let workload = RandomRestartWorkload::new(
            self.min_delay,
            self.max_delay,
            self.target_cooldown,
            self.include_validators,
        );
        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}
