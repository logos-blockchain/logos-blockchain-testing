use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use openraft_kv_node::{OpenRaftKvClient, OpenRaftKvState};
use thiserror::Error;
use tokio::time::{Instant, sleep};

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const CLIENT_RESOLUTION_INTERVAL: Duration = Duration::from_millis(200);

/// Fixed voter set used by the example cluster.
pub const FULL_VOTER_SET: [u64; 3] = [0, 1, 2];

/// One learner candidate discovered from cluster state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearnerTarget {
    /// Node identifier used by OpenRaft membership.
    pub node_id: u64,
    /// Public address advertised for Raft traffic.
    pub public_addr: String,
}

/// Membership view captured from the current node states.
#[derive(Clone, Debug)]
pub struct OpenRaftMembership {
    states: Vec<OpenRaftKvState>,
}

impl OpenRaftMembership {
    /// Reads and sorts the current node states by id.
    pub async fn discover(clients: &[OpenRaftKvClient]) -> Result<Self, OpenRaftClusterError> {
        let mut states = Vec::with_capacity(clients.len());

        for client in clients {
            states.push(client.state().await.map_err(OpenRaftClusterError::Client)?);
        }

        states.sort_by_key(|state| state.node_id);

        Ok(Self { states })
    }

    /// Returns the full voter set implied by the discovered nodes.
    #[must_use]
    pub fn voter_ids(&self) -> BTreeSet<u64> {
        self.states.iter().map(|state| state.node_id).collect()
    }

    /// Returns every non-leader node as a learner target.
    #[must_use]
    pub fn learner_targets(&self, leader_id: u64) -> Vec<LearnerTarget> {
        self.states
            .iter()
            .filter(|state| state.node_id != leader_id)
            .map(|state| LearnerTarget {
                node_id: state.node_id,
                public_addr: state.public_addr.clone(),
            })
            .collect()
    }
}

/// One poll result across all known clients.
#[derive(Clone, Debug, Default)]
pub struct OpenRaftObservation {
    states: Vec<OpenRaftKvState>,
    failures: Vec<String>,
}

impl OpenRaftObservation {
    /// Captures one best-effort view of the cluster.
    pub async fn capture(clients: &[OpenRaftKvClient]) -> Self {
        let mut states = Vec::with_capacity(clients.len());
        let mut failures = Vec::new();

        for (index, client) in clients.iter().enumerate() {
            match client.state().await {
                Ok(state) => states.push(state),
                Err(error) => failures.push(format!("client_index={index} error={error}")),
            }
        }

        states.sort_by_key(|state| state.node_id);

        Self { states, failures }
    }

    /// Returns the unique observed leader when all responding nodes agree.
    #[must_use]
    pub fn agreed_leader(&self, different_from: Option<u64>) -> Option<u64> {
        let observed = self
            .states
            .iter()
            .filter_map(|state| state.current_leader)
            .collect::<BTreeSet<_>>();

        let leader = observed.iter().next().copied()?;

        (observed.len() == 1 && different_from != Some(leader)).then_some(leader)
    }

    /// Returns `true` when every responding node reports the expected voter
    /// set.
    #[must_use]
    pub fn all_voters_match(&self, expected_voters: &BTreeSet<u64>) -> bool {
        !self.states.is_empty()
            && self.failures.is_empty()
            && self.states.iter().all(|state| {
                state.voters.iter().copied().collect::<BTreeSet<_>>() == *expected_voters
            })
    }

    /// Returns `true` when every responding node exposes the expected key/value
    /// data.
    #[must_use]
    pub fn all_kv_match(&self, expected: &BTreeMap<String, String>) -> bool {
        !self.states.is_empty()
            && self.failures.is_empty()
            && self.states.iter().all(|state| {
                state.current_leader.is_some()
                    && state.voters == FULL_VOTER_SET
                    && expected
                        .iter()
                        .all(|(key, value)| state.kv.get(key) == Some(value))
            })
    }

    /// Returns a concise summary for timeout errors.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut lines = self
            .states
            .iter()
            .map(|state| {
                format!(
                    "node={} leader={:?} voters={:?} keys={}",
                    state.node_id,
                    state.current_leader,
                    state.voters,
                    state.kv.len()
                )
            })
            .collect::<Vec<_>>();

        lines.extend(self.failures.iter().cloned());

        if lines.is_empty() {
            return "no state observed yet".to_owned();
        }

        lines.join("; ")
    }
}

/// Errors raised by the OpenRaft example cluster helpers.
#[derive(Debug, Error)]
pub enum OpenRaftClusterError {
    #[error("openraft example requires at least {expected} node clients, got {actual}")]
    InsufficientClients { expected: usize, actual: usize },
    #[error("failed to query openraft node state: {0}")]
    Client(#[source] anyhow::Error),
    #[error(
        "timed out waiting for {action} after {timeout:?}; last observation: {last_observation}"
    )]
    Timeout {
        action: &'static str,
        timeout: Duration,
        last_observation: String,
    },
    #[error("timed out resolving node client for {node_id} after {timeout:?}")]
    ClientResolution { node_id: u64, timeout: Duration },
}

/// Ensures the example cluster has the expected number of node clients.
pub fn ensure_cluster_size(
    clients: &[OpenRaftKvClient],
    expected: usize,
) -> Result<(), OpenRaftClusterError> {
    if clients.len() < expected {
        return Err(OpenRaftClusterError::InsufficientClients {
            expected,
            actual: clients.len(),
        });
    }

    Ok(())
}

/// Waits until the cluster converges on one leader.
pub async fn wait_for_leader(
    clients: &[OpenRaftKvClient],
    timeout: Duration,
    different_from: Option<u64>,
) -> Result<u64, OpenRaftClusterError> {
    let deadline = Instant::now() + timeout;

    loop {
        let last_observation = OpenRaftObservation::capture(clients).await;

        if let Some(leader) = last_observation.agreed_leader(different_from) {
            return Ok(leader);
        }

        if Instant::now() >= deadline {
            return Err(OpenRaftClusterError::Timeout {
                action: "leader agreement",
                timeout,
                last_observation: last_observation.summary(),
            });
        }

        sleep(POLL_INTERVAL).await;
    }
}

/// Waits until every node reports the expected voter set.
pub async fn wait_for_membership(
    clients: &[OpenRaftKvClient],
    expected_voters: &BTreeSet<u64>,
    timeout: Duration,
) -> Result<(), OpenRaftClusterError> {
    let deadline = Instant::now() + timeout;

    loop {
        let last_observation = OpenRaftObservation::capture(clients).await;

        if last_observation.all_voters_match(expected_voters) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(OpenRaftClusterError::Timeout {
                action: "membership convergence",
                timeout,
                last_observation: last_observation.summary(),
            });
        }

        sleep(POLL_INTERVAL).await;
    }
}

/// Waits until every node reports the full replicated key set.
pub async fn wait_for_replication(
    clients: &[OpenRaftKvClient],
    expected: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<(), OpenRaftClusterError> {
    let deadline = Instant::now() + timeout;

    loop {
        let last_observation = OpenRaftObservation::capture(clients).await;

        if last_observation.all_kv_match(expected) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(OpenRaftClusterError::Timeout {
                action: "replicated state convergence",
                timeout,
                last_observation: last_observation.summary(),
            });
        }

        sleep(POLL_INTERVAL).await;
    }
}

/// Resolves the client handle that currently identifies as `node_id`.
pub async fn resolve_client_for_node(
    clients: &[OpenRaftKvClient],
    node_id: u64,
    timeout: Duration,
) -> Result<OpenRaftKvClient, OpenRaftClusterError> {
    let deadline = Instant::now() + timeout;

    loop {
        for client in clients {
            let Ok(state) = client.state().await else {
                continue;
            };

            if state.node_id == node_id {
                return Ok(client.clone());
            }
        }

        if Instant::now() >= deadline {
            return Err(OpenRaftClusterError::ClientResolution { node_id, timeout });
        }

        sleep(CLIENT_RESOLUTION_INTERVAL).await;
    }
}

/// Issues a contiguous batch of writes through the current leader.
pub async fn write_batch(
    leader: &OpenRaftKvClient,
    prefix: &str,
    start: usize,
    count: usize,
) -> Result<(), OpenRaftClusterError> {
    for index in start..(start + count) {
        let key = format!("{prefix}-{index}");
        let value = format!("value-{index}");

        leader
            .write(&key, &value, index as u64 + 1)
            .await
            .map_err(OpenRaftClusterError::Client)?;
    }

    Ok(())
}

/// Builds the replicated key/value map expected after the workload completes.
#[must_use]
pub fn expected_kv(prefix: &str, total_writes: usize) -> BTreeMap<String, String> {
    (0..total_writes)
        .map(|index| (format!("{prefix}-{index}"), format!("value-{index}")))
        .collect()
}
