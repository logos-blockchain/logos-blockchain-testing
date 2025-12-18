use nomos_network::backends::libp2p::Libp2pInfo;
use reqwest::{Client, Url};
use thiserror::Error;
use tracing::warn;

use super::ReadinessCheck;
use crate::topology::deployment::Topology;

#[derive(Debug, Error)]
pub enum NetworkInfoError {
    #[error("failed to join url {base} with path {path}: {message}")]
    JoinUrl {
        base: Url,
        path: &'static str,
        message: String,
    },
    #[error(transparent)]
    Request(#[from] reqwest::Error),
}

#[derive(Debug)]
pub struct NodeNetworkStatus {
    label: String,
    expected_peers: Option<usize>,
    result: Result<Libp2pInfo, NetworkInfoError>,
}

pub struct NetworkReadiness<'a> {
    pub(crate) topology: &'a Topology,
    pub(crate) expected_peer_counts: &'a [usize],
    pub(crate) labels: &'a [String],
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for NetworkReadiness<'a> {
    type Data = Vec<NodeNetworkStatus>;

    async fn collect(&'a self) -> Self::Data {
        let validator_futures = self
            .topology
            .validators
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                let label = self
                    .labels
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| format!("validator#{idx}"));
                let expected_peers = self.expected_peer_counts.get(idx).copied();
                async move {
                    let result = node
                        .api()
                        .network_info()
                        .await
                        .map_err(NetworkInfoError::from);
                    NodeNetworkStatus {
                        label,
                        expected_peers,
                        result,
                    }
                }
            });
        let offset = self.topology.validators.len();
        let executor_futures = self
            .topology
            .executors
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                let global_idx = offset + idx;
                let label = self
                    .labels
                    .get(global_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("executor#{idx}"));
                let expected_peers = self.expected_peer_counts.get(global_idx).copied();
                async move {
                    let result = node
                        .api()
                        .network_info()
                        .await
                        .map_err(NetworkInfoError::from);
                    NodeNetworkStatus {
                        label,
                        expected_peers,
                        result,
                    }
                }
            });

        let (validator_statuses, executor_statuses) = tokio::join!(
            futures::future::join_all(validator_futures),
            futures::future::join_all(executor_futures)
        );
        validator_statuses
            .into_iter()
            .chain(executor_statuses)
            .collect()
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter().all(
            |status| match (status.expected_peers, status.result.as_ref()) {
                (Some(expected), Ok(info)) => info.n_peers >= expected,
                _ => false,
            },
        )
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = build_timeout_summary(&data);
        format!("timed out waiting for network readiness: {summary}")
    }
}

pub struct HttpNetworkReadiness<'a> {
    pub(crate) client: &'a Client,
    pub(crate) endpoints: &'a [Url],
    pub(crate) expected_peer_counts: &'a [usize],
    pub(crate) labels: &'a [String],
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for HttpNetworkReadiness<'a> {
    type Data = Vec<NodeNetworkStatus>;

    async fn collect(&'a self) -> Self::Data {
        let futures = self.endpoints.iter().enumerate().map(|(idx, endpoint)| {
            let label = self
                .labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("endpoint#{idx}"));
            let expected_peers = self.expected_peer_counts.get(idx).copied();
            async move {
                let result = try_fetch_network_info(self.client, endpoint).await;
                NodeNetworkStatus {
                    label,
                    expected_peers,
                    result,
                }
            }
        });
        futures::future::join_all(futures).await
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter().all(
            |status| match (status.expected_peers, status.result.as_ref()) {
                (Some(expected), Ok(info)) => info.n_peers >= expected,
                _ => false,
            },
        )
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = build_timeout_summary(&data);
        format!("timed out waiting for network readiness: {summary}")
    }
}

pub async fn try_fetch_network_info(
    client: &Client,
    base: &Url,
) -> Result<Libp2pInfo, NetworkInfoError> {
    let path = nomos_http_api_common::paths::NETWORK_INFO.trim_start_matches('/');
    let url = base
        .join(path)
        .map_err(|source| NetworkInfoError::JoinUrl {
            base: base.clone(),
            path: nomos_http_api_common::paths::NETWORK_INFO,
            message: source.to_string(),
        })?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(NetworkInfoError::Request)?
        .error_for_status()
        .map_err(NetworkInfoError::Request)?;

    response
        .json::<Libp2pInfo>()
        .await
        .map_err(NetworkInfoError::Request)
}

#[deprecated(note = "use try_fetch_network_info to avoid panics and preserve error details")]
pub async fn fetch_network_info(client: &Client, base: &Url) -> Libp2pInfo {
    match try_fetch_network_info(client, base).await {
        Ok(info) => info,
        Err(err) => log_network_warning(base, &err),
    }
}

fn log_network_warning(base: &Url, err: &NetworkInfoError) -> Libp2pInfo {
    warn!(
        target: "readiness",
        url = %base,
        error = %err,
        "network readiness: failed to fetch network info"
    );
    empty_libp2p_info()
}

fn empty_libp2p_info() -> Libp2pInfo {
    Libp2pInfo {
        listen_addresses: Vec::with_capacity(0),
        n_peers: 0,
        n_connections: 0,
        n_pending_connections: 0,
    }
}

fn build_timeout_summary(statuses: &[NodeNetworkStatus]) -> String {
    statuses
        .iter()
        .map(
            |status| match (status.expected_peers, status.result.as_ref()) {
                (None, _) => format!("{}: missing expected peer count", status.label),
                (Some(expected), Ok(info)) => {
                    format!(
                        "{}: peers={}, expected={}",
                        status.label, info.n_peers, expected
                    )
                }
                (Some(expected), Err(err)) => {
                    format!("{}: error={err}, expected_peers={expected}", status.label)
                }
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}
