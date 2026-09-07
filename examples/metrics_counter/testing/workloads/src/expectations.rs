use async_trait::async_trait;
use metrics_counter_runtime_ext::MetricsCounterEnv;
use testing_framework_app::{AppHostEnv, AppRunContextExt as _};
use testing_framework_core::scenario::{ClusterHandle, DynError, Expectation, RunContext};
use tracing::info;

#[derive(Clone)]
pub struct PrometheusCounterAtLeast {
    min_total: f64,
}

impl PrometheusCounterAtLeast {
    #[must_use]
    pub const fn new(min_total: f64) -> Self {
        Self { min_total }
    }
}

#[async_trait]
impl Expectation<AppHostEnv> for PrometheusCounterAtLeast {
    fn name(&self) -> &str {
        "prometheus_counter_at_least"
    }

    async fn evaluate(&mut self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let cluster = ctx.require_app::<ClusterHandle<MetricsCounterEnv>>()?;
        let metrics = cluster.metrics()?;
        if !metrics.is_configured() {
            return Err(
                "prometheus endpoint unavailable; set LOGOS_BLOCKCHAIN_METRICS_QUERY_URL".into(),
            );
        }

        let total = metrics.counter_value("sum(metrics_counter_increments_total)")?;

        if total < self.min_total {
            return Err(format!(
                "metrics_counter_increments_total below threshold: total={total}, min={}",
                self.min_total
            )
            .into());
        }

        info!(
            total,
            min = self.min_total,
            "prometheus counter threshold met"
        );
        Ok(())
    }
}
