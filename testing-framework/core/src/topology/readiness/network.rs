use nomos_network::backends::libp2p::Libp2pInfo;
use reqwest::{Client, Url};
use tracing::warn;

use super::ReadinessCheck;
use crate::topology::Topology;

pub struct NetworkReadiness<'a> {
    pub(crate) topology: &'a Topology,
    pub(crate) expected_peer_counts: &'a [usize],
    pub(crate) labels: &'a [String],
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for NetworkReadiness<'a> {
    type Data = Vec<Libp2pInfo>;

    async fn collect(&'a self) -> Self::Data {
        let (validator_infos, executor_infos) = tokio::join!(
            futures::future::join_all(
                self.topology
                    .validators
                    .iter()
                    .map(crate::nodes::validator::Validator::network_info)
            ),
            futures::future::join_all(
                self.topology
                    .executors
                    .iter()
                    .map(crate::nodes::executor::Executor::network_info)
            )
        );

        validator_infos.into_iter().chain(executor_infos).collect()
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter()
            .enumerate()
            .all(|(idx, info)| info.n_peers >= self.expected_peer_counts[idx])
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = build_timeout_summary(self.labels, data, self.expected_peer_counts);
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
    type Data = Vec<Libp2pInfo>;

    async fn collect(&'a self) -> Self::Data {
        let futures = self
            .endpoints
            .iter()
            .map(|endpoint| fetch_network_info(self.client, endpoint));
        futures::future::join_all(futures).await
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter()
            .enumerate()
            .all(|(idx, info)| info.n_peers >= self.expected_peer_counts[idx])
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = build_timeout_summary(self.labels, data, self.expected_peer_counts);
        format!("timed out waiting for network readiness: {summary}")
    }
}

async fn fetch_network_info(client: &Client, base: &Url) -> Libp2pInfo {
    let url = base
        .join(nomos_http_api_common::paths::NETWORK_INFO.trim_start_matches('/'))
        .unwrap_or_else(|err| {
            panic!(
                "failed to join url {base} with path {}: {err}",
                nomos_http_api_common::paths::NETWORK_INFO
            )
        });
    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return log_network_warning(base, err, "failed to reach network info endpoint");
        }
    };

    let response = match response.error_for_status() {
        Ok(resp) => resp,
        Err(err) => {
            return log_network_warning(base, err, "network info endpoint returned error");
        }
    };

    match response.json::<Libp2pInfo>().await {
        Ok(info) => info,
        Err(err) => log_network_warning(base, err, "failed to decode network info response"),
    }
}

fn log_network_warning(base: &Url, err: impl std::fmt::Display, message: &str) -> Libp2pInfo {
    warn!(target: "readiness", url = %base, error = %err, "{message}");
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

fn build_timeout_summary(
    labels: &[String],
    infos: Vec<Libp2pInfo>,
    expected_counts: &[usize],
) -> String {
    infos
        .into_iter()
        .zip(expected_counts.iter())
        .zip(labels.iter())
        .map(|((info, expected), label)| {
            format!("{}: peers={}, expected={}", label, info.n_peers, expected)
        })
        .collect::<Vec<_>>()
        .join(", ")
}
