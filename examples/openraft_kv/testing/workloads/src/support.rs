use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use openraft_kv_node::{OpenRaftKvClient, OpenRaftKvState};
use openraft_kv_runtime_ext::{
    OpenRaftClusterObserver, OpenRaftClusterSnapshot, capture_openraft_cluster_snapshot,
};
use testing_framework_core::observation::{ObservationHandle, ObservationSnapshot};
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
    /// Builds a membership view from already observed node states.
    #[must_use]
    pub fn from_states(states: &[OpenRaftKvState]) -> Self {
        let mut states = states.to_vec();
        states.sort_by_key(|state| state.node_id);

        Self { states }
    }

    /// Reads and sorts the current node states by id.
    pub async fn discover(clients: &[OpenRaftKvClient]) -> Result<Self, OpenRaftClusterError> {
        let mut states = Vec::with_capacity(clients.len());

        for client in clients {
            states.push(client.state().await.map_err(OpenRaftClusterError::Client)?);
        }

        Ok(Self::from_states(&states))
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

/// Errors raised by the OpenRaft example cluster helpers.
#[derive(Debug, Error)]
pub enum OpenRaftClusterError {
    #[error("openraft example requires at least {expected} node clients, got {actual}")]
    InsufficientClients { expected: usize, actual: usize },
    #[error("failed to query openraft node state: {0}")]
    Client(#[source] anyhow::Error),
    #[error("openraft cluster observation is not available yet")]
    MissingObservation,
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
        let last_observation = capture_openraft_cluster_snapshot(clients).await;

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
        let last_observation = capture_openraft_cluster_snapshot(clients).await;

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
        let last_observation = capture_openraft_cluster_snapshot(clients).await;

        if last_observation.all_kv_match(expected, &FULL_VOTER_SET) {
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

/// Waits until the observer reports one agreed leader.
pub async fn wait_for_observed_leader(
    handle: &ObservationHandle<OpenRaftClusterObserver>,
    timeout: Duration,
    different_from: Option<u64>,
) -> Result<u64, OpenRaftClusterError> {
    let snapshot =
        wait_for_observed_snapshot(handle, timeout, "observed leader agreement", |snapshot| {
            snapshot.agreed_leader(different_from).is_some()
        })
        .await?;

    snapshot
        .value
        .agreed_leader(different_from)
        .ok_or(OpenRaftClusterError::MissingObservation)
}

/// Waits until the observer reports the expected voter set on every node.
pub async fn wait_for_observed_membership(
    handle: &ObservationHandle<OpenRaftClusterObserver>,
    expected_voters: &BTreeSet<u64>,
    timeout: Duration,
) -> Result<(), OpenRaftClusterError> {
    wait_for_observed_snapshot(
        handle,
        timeout,
        "observed membership convergence",
        |snapshot| snapshot.all_voters_match(expected_voters),
    )
    .await?;

    Ok(())
}

/// Waits until the observer reports the full replicated key set.
pub async fn wait_for_observed_replication(
    handle: &ObservationHandle<OpenRaftClusterObserver>,
    expected: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<(), OpenRaftClusterError> {
    wait_for_observed_snapshot(
        handle,
        timeout,
        "observed replicated state convergence",
        |snapshot| snapshot.all_kv_match(expected, &FULL_VOTER_SET),
    )
    .await?;

    Ok(())
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

async fn wait_for_observed_snapshot(
    handle: &ObservationHandle<OpenRaftClusterObserver>,
    timeout: Duration,
    action: &'static str,
    matches: impl Fn(&OpenRaftClusterSnapshot) -> bool,
) -> Result<ObservationSnapshot<OpenRaftClusterSnapshot>, OpenRaftClusterError> {
    let deadline = Instant::now() + timeout;
    let mut last_summary = "no state observed yet".to_owned();

    loop {
        if let Some(snapshot) = handle.latest_snapshot() {
            last_summary = snapshot.value.summary();

            if matches(&snapshot.value) {
                return Ok(snapshot);
            }
        }

        if Instant::now() >= deadline {
            return Err(OpenRaftClusterError::Timeout {
                action,
                timeout,
                last_observation: last_summary,
            });
        }

        sleep(POLL_INTERVAL).await;
    }
}
