use std::time::Duration;

use lb_network_service::backends::libp2p::Libp2pInfo;
use testing_framework_core::topology::readiness::ReadinessCheck;
use tokio::time::timeout;

use crate::node_control::ReadinessNode;

const NETWORK_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct ManualNetworkReadiness {
    nodes: Vec<ReadinessNode>,
}

impl ManualNetworkReadiness {
    pub(super) fn new(nodes: Vec<ReadinessNode>) -> Self {
        Self { nodes }
    }
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

pub(super) struct ManualNetworkStatus {
    label: String,
    expected_peers: Option<usize>,
    result: Result<Libp2pInfo, String>,
}
