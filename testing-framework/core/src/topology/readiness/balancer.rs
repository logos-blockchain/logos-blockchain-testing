use nomos_da_network_core::swarm::BalancerStats;

use super::ReadinessCheck;
use crate::topology::deployment::Topology;

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug)]
pub struct NodeBalancerStatus {
    label: String,
    threshold: usize,
    result: Result<BalancerStats, reqwest::Error>,
}

pub struct DaBalancerReadiness<'a> {
    pub(crate) topology: &'a Topology,
    pub(crate) labels: &'a [String],
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for DaBalancerReadiness<'a> {
    type Data = Vec<NodeBalancerStatus>;

    async fn collect(&'a self) -> Self::Data {
        let mut data = Vec::new();
        for (idx, node) in self.topology.nodes.iter().enumerate() {
            let label = self
                .labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("node#{idx}"));
            data.push(
                (
                    label,
                    node.config().da_network.subnet_threshold,
                    node.api().balancer_stats().await,
                )
                    .into(),
            );
        }
        data
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        if self.topology.nodes.len() <= 1 {
            return true;
        }
        data.iter().all(|entry| {
            if entry.threshold == 0 {
                return true;
            }
            entry
                .result
                .as_ref()
                .is_ok_and(|stats| connected_subnetworks(stats) >= entry.threshold)
        })
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = data
            .into_iter()
            .map(|entry| {
                let (connected, details, error) = match entry.result {
                    Ok(stats) => (
                        connected_subnetworks(&stats),
                        format_balancer_stats(&stats),
                        None,
                    ),
                    Err(err) => (0, "unavailable".to_string(), Some(err.to_string())),
                };
                let mut msg = format!(
                    "{}: connected={connected}, required={}, stats={details}",
                    entry.label, entry.threshold
                );
                if let Some(error) = error {
                    msg.push_str(&format!(", error={error}"));
                }
                msg
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("timed out waiting for DA balancer readiness: {summary}")
    }

    fn poll_interval(&self) -> std::time::Duration {
        POLL_INTERVAL
    }
}

fn connected_subnetworks(stats: &BalancerStats) -> usize {
    stats
        .values()
        .filter(|stat| stat.inbound > 0 || stat.outbound > 0)
        .count()
}

fn format_balancer_stats(stats: &BalancerStats) -> String {
    if stats.is_empty() {
        return "empty".into();
    }
    stats
        .iter()
        .map(|(subnet, stat)| format!("{}:in={},out={}", subnet, stat.inbound, stat.outbound))
        .collect::<Vec<_>>()
        .join(";")
}

impl From<(String, usize, Result<BalancerStats, reqwest::Error>)> for NodeBalancerStatus {
    fn from(value: (String, usize, Result<BalancerStats, reqwest::Error>)) -> Self {
        let (label, threshold, result) = value;
        Self {
            label,
            threshold,
            result,
        }
    }
}
