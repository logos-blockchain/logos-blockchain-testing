use nomos_da_network_core::swarm::BalancerStats;

use super::ReadinessCheck;
use crate::topology::Topology;

pub struct DaBalancerReadiness<'a> {
    pub(crate) topology: &'a Topology,
    pub(crate) labels: &'a [String],
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for DaBalancerReadiness<'a> {
    type Data = Vec<(String, usize, BalancerStats)>;

    async fn collect(&'a self) -> Self::Data {
        let mut data = Vec::new();
        for (idx, validator) in self.topology.validators.iter().enumerate() {
            data.push((
                self.labels[idx].clone(),
                validator.config().da_network.subnet_threshold,
                validator.balancer_stats().await,
            ));
        }
        for (offset, executor) in self.topology.executors.iter().enumerate() {
            let label_index = self.topology.validators.len() + offset;
            data.push((
                self.labels[label_index].clone(),
                executor.config().da_network.subnet_threshold,
                executor.balancer_stats().await,
            ));
        }
        data
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter().all(|(_, threshold, stats)| {
            if *threshold == 0 {
                return true;
            }
            connected_subnetworks(stats) >= *threshold
        })
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = data
            .into_iter()
            .map(|(label, threshold, stats)| {
                let connected = connected_subnetworks(&stats);
                let details = format_balancer_stats(&stats);
                format!("{label}: connected={connected}, required={threshold}, stats={details}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("timed out waiting for DA balancer readiness: {summary}")
    }

    fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
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
