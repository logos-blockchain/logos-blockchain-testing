use std::env;

use reqwest::Url;

use super::{Metrics, MetricsError};

/// Optional observability endpoints used by deployers and runners.
#[derive(Clone, Debug, Default)]
pub struct ObservabilityInputs {
    /// Base URL used by the runner to query Prometheus.
    pub metrics_query_url: Option<Url>,
    /// OTLP HTTP endpoint used by nodes to export metrics.
    pub metrics_otlp_ingest_url: Option<Url>,
    /// Optional Grafana URL for logs/output.
    pub grafana_url: Option<Url>,
}

impl ObservabilityInputs {
    /// Load observability inputs from `LOGOS_BLOCKCHAIN_*` environment vars.
    pub fn from_env() -> Result<Self, MetricsError> {
        Ok(Self {
            metrics_query_url: read_url_var(&["LOGOS_BLOCKCHAIN_METRICS_QUERY_URL"])?,
            metrics_otlp_ingest_url: read_url_var(&["LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL"])?,
            grafana_url: read_url_var(&["LOGOS_BLOCKCHAIN_GRAFANA_URL"])?,
        })
    }

    /// Override `self` values with non-empty values from `overrides`.
    #[must_use]
    pub fn with_overrides(mut self, overrides: Self) -> Self {
        if overrides.metrics_query_url.is_some() {
            self.metrics_query_url = overrides.metrics_query_url;
        }

        if overrides.metrics_otlp_ingest_url.is_some() {
            self.metrics_otlp_ingest_url = overrides.metrics_otlp_ingest_url;
        }

        if overrides.grafana_url.is_some() {
            self.grafana_url = overrides.grafana_url;
        }
        self
    }

    /// Build the telemetry handle used in `RunContext`.
    pub fn telemetry_handle(&self) -> Result<Metrics, MetricsError> {
        match self.metrics_query_url.clone() {
            Some(url) => Metrics::from_prometheus(url),
            None => Ok(Metrics::empty()),
        }
    }
}

fn read_url_var(keys: &[&'static str]) -> Result<Option<Url>, MetricsError> {
    for key in keys {
        let Some(raw) = env::var(key).ok() else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        return Url::parse(raw)
            .map(Some)
            .map_err(|err| MetricsError::new(format!("invalid {key}: {err}")));
    }

    Ok(None)
}
