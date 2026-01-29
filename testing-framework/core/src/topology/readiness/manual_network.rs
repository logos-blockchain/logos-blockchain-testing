use std::{collections::HashSet, time::Duration};

use nomos_network::backends::libp2p::Libp2pInfo;
use tokio::time::timeout;

use super::ReadinessCheck;
use crate::{nodes::ApiClient, topology::generation::find_expected_peer_counts};

const NETWORK_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ReadinessNode {
    pub label: String,
    pub expected_peers: Option<usize>,
    pub api: ApiClient,
}

pub struct ManualNetworkReadiness {
    nodes: Vec<ReadinessNode>,
}

impl ManualNetworkReadiness {
    pub fn new(nodes: Vec<ReadinessNode>) -> Self {
        Self { nodes }
    }
}

pub struct ManualNetworkStatus {
    label: String,
    expected_peers: Option<usize>,
    result: Result<Libp2pInfo, String>,
}

pub fn build_readiness_nodes<I>(iter: I) -> Vec<ReadinessNode>
where
    I: IntoIterator<Item = (String, ApiClient, u16, HashSet<u16>)>,
{
    let entries = iter.into_iter().collect::<Vec<_>>();
    let listen_ports = entries.iter().map(|entry| entry.2).collect::<Vec<_>>();

    let initial_peer_ports = entries
        .iter()
        .map(|entry| entry.3.clone())
        .collect::<Vec<_>>();

    let expected_peer_counts = find_expected_peer_counts(&listen_ports, &initial_peer_ports);

    entries
        .into_iter()
        .enumerate()
        .map(|(idx, (label, api, _, _))| ReadinessNode {
            label,
            expected_peers: expected_peer_counts.get(idx).copied(),
            api,
        })
        .collect()
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for ManualNetworkReadiness {
    type Data = Vec<ManualNetworkStatus>;

    async fn collect(&'a self) -> Self::Data {
        let mut statuses = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let result = timeout(NETWORK_REQUEST_TIMEOUT, node.api.network_info())
                .await
                .map_err(|_| "network_info request timed out".to_owned())
                .and_then(|res| res.map_err(|err| err.to_string()));

            statuses.push(ManualNetworkStatus {
                label: node.label.clone(),
                expected_peers: node.expected_peers,
                result,
            });
        }
        statuses
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
        let summary = data
            .into_iter()
            .map(|entry| match entry.result {
                Ok(info) => format!(
                    "{} (peers {}/{})",
                    entry.label,
                    info.n_peers,
                    entry.expected_peers.unwrap_or(0)
                ),
                Err(err) => format!("{} (error: {err})", entry.label),
            })
            .collect::<Vec<_>>()
            .join(", ");

        format!("timed out waiting for network readiness: {summary}")
    }
}
