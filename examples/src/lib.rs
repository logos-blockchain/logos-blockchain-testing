use testing_framework_core::scenario::Metrics;
pub use testing_framework_workflows::{
    builder::{ChaosBuilderExt, ScenarioBuilderExt},
    expectations, util, workloads,
};

pub mod env;

pub use env::read_env_any;

/// Metrics are currently disabled in this branch; return a stub handle.
#[must_use]
pub const fn configure_prometheus_metrics() -> Metrics {
    Metrics::empty()
}
