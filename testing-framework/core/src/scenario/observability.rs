use std::env;

use reqwest::Url;

use super::{Metrics, MetricsError, NodeControlCapability, ObservabilityCapability};

/// Observability configuration inputs shared by deployers/runners.
///
/// All fields are optional; missing values only matter when a caller needs the
/// corresponding capability (e.g. querying metrics from the runner process).
#[derive(Clone, Debug, Default)]
pub struct ObservabilityInputs {
    /// Prometheus-compatible base URL used by the runner process to query
    /// metrics (PromQL API endpoints).
    pub metrics_query_url: Option<Url>,
    /// Prometheus-compatible base URL intended for an in-cluster Grafana
    /// datasource.
    pub metrics_query_grafana_url: Option<Url>,
    /// Full OTLP HTTP metrics ingest endpoint used by nodes to export metrics
    /// (backend-specific host and path).
    pub metrics_otlp_ingest_url: Option<Url>,
    /// Optional Grafana base URL for printing/logging (human access).
    pub grafana_url: Option<Url>,
}

/// Capability helper for deployers that are generic over scenario capability
/// markers.
pub trait ObservabilityCapabilityProvider {
    fn observability_capability(&self) -> Option<&ObservabilityCapability>;
}

impl ObservabilityCapabilityProvider for () {
    fn observability_capability(&self) -> Option<&ObservabilityCapability> {
        None
    }
}

impl ObservabilityCapabilityProvider for NodeControlCapability {
    fn observability_capability(&self) -> Option<&ObservabilityCapability> {
        None
    }
}

impl ObservabilityCapabilityProvider for ObservabilityCapability {
    fn observability_capability(&self) -> Option<&ObservabilityCapability> {
        Some(self)
    }
}

impl ObservabilityInputs {
    #[must_use]
    pub fn from_capability(capabilities: &ObservabilityCapability) -> Self {
        Self {
            metrics_query_url: capabilities.metrics_query_url.clone(),
            metrics_query_grafana_url: capabilities.metrics_query_grafana_url.clone(),
            metrics_otlp_ingest_url: capabilities.metrics_otlp_ingest_url.clone(),
            grafana_url: capabilities.grafana_url.clone(),
        }
    }

    /// Load observability inputs from environment variables.
    ///
    /// The `NOMOS_*` namespace applies to all deployers. Runner-specific env
    /// vars are also accepted as aliases for backwards compatibility.
    pub fn from_env() -> Result<Self, MetricsError> {
        Ok(Self {
            metrics_query_url: read_url_var(&[
                "NOMOS_METRICS_QUERY_URL",
                "K8S_RUNNER_METRICS_QUERY_URL",
                // Back-compat:
                "K8S_RUNNER_EXTERNAL_PROMETHEUS_URL",
                "NOMOS_EXTERNAL_PROMETHEUS_URL",
            ])?,
            metrics_query_grafana_url: read_url_var(&[
                "NOMOS_METRICS_QUERY_GRAFANA_URL",
                "K8S_RUNNER_METRICS_QUERY_GRAFANA_URL",
                // Back-compat:
                "K8S_RUNNER_EXTERNAL_PROMETHEUS_GRAFANA_URL",
                "NOMOS_EXTERNAL_PROMETHEUS_GRAFANA_URL",
            ])?,
            metrics_otlp_ingest_url: read_url_var(&[
                "NOMOS_METRICS_OTLP_INGEST_URL",
                "K8S_RUNNER_METRICS_OTLP_INGEST_URL",
                // Back-compat:
                "K8S_RUNNER_EXTERNAL_OTLP_METRICS_ENDPOINT",
                "NOMOS_EXTERNAL_OTLP_METRICS_ENDPOINT",
            ])?,
            grafana_url: read_url_var(&["NOMOS_GRAFANA_URL", "K8S_RUNNER_GRAFANA_URL"])?,
        })
    }

    /// Apply defaults and fallbacks (pure function).
    ///
    /// Currently, the only fallback is using `metrics_query_url` as the Grafana
    /// datasource URL when `metrics_query_grafana_url` is unset.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        if self.metrics_query_grafana_url.is_none() {
            self.metrics_query_grafana_url = self.metrics_query_url.clone();
        }
        self
    }

    /// Overlay non-empty values from `overrides` onto `self`.
    #[must_use]
    pub fn with_overrides(mut self, overrides: Self) -> Self {
        if overrides.metrics_query_url.is_some() {
            self.metrics_query_url = overrides.metrics_query_url;
        }
        if overrides.metrics_query_grafana_url.is_some() {
            self.metrics_query_grafana_url = overrides.metrics_query_grafana_url;
        }
        if overrides.metrics_otlp_ingest_url.is_some() {
            self.metrics_otlp_ingest_url = overrides.metrics_otlp_ingest_url;
        }
        if overrides.grafana_url.is_some() {
            self.grafana_url = overrides.grafana_url;
        }
        self
    }

    /// Build the telemetry handle exposed in `RunContext::telemetry()`.
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
