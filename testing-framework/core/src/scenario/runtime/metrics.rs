use std::{collections::HashMap, sync::Arc, thread};

use prometheus_http_query::{
    Client as PrometheusClient,
    response::{Data as PrometheusData, PromqlResult, Sample},
};
use reqwest::Url;
use tokio::runtime::Runtime;
use tracing::warn;

pub const CONSENSUS_PROCESSED_BLOCKS: &str = "consensus_processed_blocks";
pub const CONSENSUS_TRANSACTIONS_TOTAL: &str = "consensus_transactions_total";
const CONSENSUS_TRANSACTIONS_NODE_QUERY: &str =
    r#"sum(consensus_transactions_total{job=~"node-.*"})"#;
const NODE_QUERY_FALLBACK_MESSAGE: &str = "falling back to aggregate consensus transaction counter";

/// Telemetry handles available during a run.
#[derive(Clone, Default)]
pub struct Metrics {
    prometheus: Option<Arc<PrometheusEndpoint>>,
}

impl Metrics {
    #[must_use]
    pub const fn empty() -> Self {
        Self { prometheus: None }
    }

    pub fn from_prometheus(url: Url) -> Result<Self, MetricsError> {
        let handle = Arc::new(PrometheusEndpoint::new(url)?);
        Ok(Self::empty().with_prometheus_endpoint(handle))
    }

    pub fn from_prometheus_str(raw_url: &str) -> Result<Self, MetricsError> {
        Url::parse(raw_url)
            .map_err(|err| MetricsError::new(format!("invalid prometheus url: {err}")))
            .and_then(Self::from_prometheus)
    }

    #[must_use]
    pub fn with_prometheus_endpoint(mut self, handle: Arc<PrometheusEndpoint>) -> Self {
        self.prometheus = Some(handle);
        self
    }

    #[must_use]
    pub fn prometheus(&self) -> Option<Arc<PrometheusEndpoint>> {
        self.prometheus.as_ref().map(Arc::clone)
    }

    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.prometheus.is_some()
    }

    pub fn instant_values(&self, query: &str) -> Result<Vec<f64>, MetricsError> {
        self.require_prometheus()?.instant_values(query)
    }

    pub fn counter_value(&self, query: &str) -> Result<f64, MetricsError> {
        self.require_prometheus()?.counter_value(query)
    }

    pub fn consensus_processed_blocks(&self) -> Result<f64, MetricsError> {
        self.counter_value(CONSENSUS_PROCESSED_BLOCKS)
    }

    pub fn consensus_transactions_total(&self) -> Result<f64, MetricsError> {
        let handle = self.require_prometheus()?;

        if let Some(total) = query_node_transactions_total(&handle) {
            return Ok(total);
        }

        handle.counter_value(CONSENSUS_TRANSACTIONS_TOTAL)
    }

    fn require_prometheus(&self) -> Result<Arc<PrometheusEndpoint>, MetricsError> {
        self.prometheus()
            .ok_or_else(|| MetricsError::new("prometheus endpoint unavailable"))
    }
}

fn query_node_transactions_total(handle: &PrometheusEndpoint) -> Option<f64> {
    match handle.instant_samples(CONSENSUS_TRANSACTIONS_NODE_QUERY) {
        Ok(samples) => samples_total_or_warn(samples),
        Err(error) => {
            warn_node_query_fallback(Some(&error));
            None
        }
    }
}

fn samples_total_or_warn(samples: Vec<PrometheusInstantSample>) -> Option<f64> {
    if samples.is_empty() {
        warn_node_query_fallback(None);
        return None;
    }

    Some(samples.into_iter().map(|sample| sample.value).sum())
}

fn warn_node_query_fallback(error: Option<&MetricsError>) {
    match error {
        Some(error) => warn!(
            query = CONSENSUS_TRANSACTIONS_NODE_QUERY,
            error = %error,
            "{NODE_QUERY_FALLBACK_MESSAGE}"
        ),
        None => warn!(
            query = CONSENSUS_TRANSACTIONS_NODE_QUERY,
            "{NODE_QUERY_FALLBACK_MESSAGE}"
        ),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("{0}")]
    Store(String),
}

impl MetricsError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self::Store(message.into())
    }
}

/// Lightweight wrapper around a Prometheus endpoint used by the framework.
pub struct PrometheusEndpoint {
    base_url: Url,
    client: PrometheusClient,
}

/// Single sample from a Prometheus instant query.
#[derive(Clone, Debug)]
pub struct PrometheusInstantSample {
    pub labels: HashMap<String, String>,
    pub timestamp: f64,
    pub value: f64,
}

impl PrometheusEndpoint {
    pub fn new(base_url: Url) -> Result<Self, MetricsError> {
        let client = PrometheusClient::try_from(base_url.as_str().to_owned()).map_err(|err| {
            MetricsError::new(format!("failed to create prometheus client: {err}"))
        })?;

        Ok(Self { base_url, client })
    }

    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.base_url.port_or_known_default()
    }

    pub fn instant_samples(
        &self,
        query: &str,
    ) -> Result<Vec<PrometheusInstantSample>, MetricsError> {
        query_prometheus(self.client.clone(), query.to_owned())
            .map(|response| samples_from_prometheus_data(response.data()))
    }

    pub fn instant_values(&self, query: &str) -> Result<Vec<f64>, MetricsError> {
        self.instant_samples(query)
            .map(|samples| samples.into_iter().map(|sample| sample.value).collect())
    }

    pub fn counter_value(&self, query: &str) -> Result<f64, MetricsError> {
        self.instant_values(query)
            .map(|values| values.into_iter().sum())
    }
}

fn query_prometheus(client: PrometheusClient, query: String) -> Result<PromqlResult, MetricsError> {
    thread::spawn(move || -> Result<_, MetricsError> {
        let runtime = Runtime::new()
            .map_err(|error| MetricsError::new(format!("failed to create runtime: {error}")))?;
        runtime
            .block_on(async { client.query(&query).get().await })
            .map_err(|error| MetricsError::new(format!("prometheus query failed: {error}")))
    })
    .join()
    .map_err(|_| MetricsError::new("prometheus query thread panicked"))?
}

fn samples_from_prometheus_data(data: &PrometheusData) -> Vec<PrometheusInstantSample> {
    let mut samples = Vec::new();

    match data {
        PrometheusData::Vector(vectors) => {
            samples.extend(vectors.iter().map(|vector| PrometheusInstantSample {
                labels: vector.metric().clone(),
                timestamp: vector.sample().timestamp(),
                value: vector.sample().value(),
            }));
        }
        PrometheusData::Matrix(ranges) => {
            for range in ranges {
                let labels = range.metric().clone();
                samples.extend(
                    range
                        .samples()
                        .iter()
                        .map(|sample| PrometheusInstantSample {
                            labels: labels.clone(),
                            timestamp: sample.timestamp(),
                            value: sample.value(),
                        }),
                );
            }
        }
        PrometheusData::Scalar(sample) => samples.push(scalar_sample(sample)),
    }

    samples
}

fn scalar_sample(sample: &Sample) -> PrometheusInstantSample {
    PrometheusInstantSample {
        labels: HashMap::new(),
        timestamp: sample.timestamp(),
        value: sample.value(),
    }
}
