use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use async_trait::async_trait;
use openraft_kv_node::{OpenRaftKvClient, OpenRaftKvState};
use testing_framework_core::{
    observation::{
        BoxedSourceProvider, ObservationConfig, ObservedSource, Observer, StaticSourceProvider,
    },
    scenario::{Application, DynError, NodeClients},
};

use crate::OpenRaftKvEnv;

const OBSERVATION_INTERVAL: Duration = Duration::from_millis(250);
const OBSERVATION_HISTORY_LIMIT: usize = 16;

/// Materialized OpenRaft cluster state built from the latest node polls.
#[derive(Clone, Debug, Default)]
pub struct OpenRaftClusterSnapshot {
    states: Vec<OpenRaftKvState>,
    failures: Vec<OpenRaftSourceFailure>,
}

impl OpenRaftClusterSnapshot {
    /// Returns the successfully observed node states sorted by node id.
    #[must_use]
    pub fn states(&self) -> &[OpenRaftKvState] {
        &self.states
    }

    /// Returns `true` when the snapshot contains no successful node states.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
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

    /// Returns `true` when every observed node reports the expected voter set.
    #[must_use]
    pub fn all_voters_match(&self, expected_voters: &BTreeSet<u64>) -> bool {
        !self.states.is_empty()
            && self.failures.is_empty()
            && self.states.iter().all(|state| {
                state.voters.iter().copied().collect::<BTreeSet<_>>() == *expected_voters
            })
    }

    /// Returns `true` when every observed node exposes the expected replicated
    /// key/value data.
    #[must_use]
    pub fn all_kv_match(
        &self,
        expected: &BTreeMap<String, String>,
        full_voter_set: &[u64],
    ) -> bool {
        !self.states.is_empty()
            && self.failures.is_empty()
            && self.states.iter().all(|state| {
                state.current_leader.is_some()
                    && state.voters == full_voter_set
                    && expected
                        .iter()
                        .all(|(key, value)| state.kv.get(key) == Some(value))
            })
    }

    /// Returns a concise summary for timeout and validation errors.
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

        lines.extend(self.failures.iter().map(OpenRaftSourceFailure::summary));

        if lines.is_empty() {
            return "no state observed yet".to_owned();
        }

        lines.join("; ")
    }
}

/// One failed source read captured during an observation cycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRaftSourceFailure {
    source_name: String,
    message: String,
}

impl OpenRaftSourceFailure {
    fn new(source_name: &str, message: &str) -> Self {
        Self {
            source_name: source_name.to_owned(),
            message: message.to_owned(),
        }
    }

    fn summary(&self) -> String {
        format!("source={} error={}", self.source_name, self.message)
    }
}

/// Observer that keeps the latest per-node OpenRaft state.
#[derive(Clone, Debug, Default)]
pub struct OpenRaftClusterObserver;

impl OpenRaftClusterObserver {
    /// Default runtime configuration for the OpenRaft example observer.
    #[must_use]
    pub fn config() -> ObservationConfig {
        ObservationConfig {
            interval: OBSERVATION_INTERVAL,
            history_limit: OBSERVATION_HISTORY_LIMIT,
        }
    }
}

/// Captures one best-effort OpenRaft cluster snapshot from the provided node
/// clients.
pub async fn capture_openraft_cluster_snapshot(
    clients: &[OpenRaftKvClient],
) -> OpenRaftClusterSnapshot {
    capture_cluster_snapshot(&named_sources(clients.to_vec())).await
}

#[async_trait]
impl Observer for OpenRaftClusterObserver {
    type Source = OpenRaftKvClient;
    type State = OpenRaftClusterSnapshot;
    type Snapshot = OpenRaftClusterSnapshot;
    type Event = ();

    async fn init(
        &self,
        sources: &[ObservedSource<Self::Source>],
    ) -> Result<Self::State, DynError> {
        Ok(capture_cluster_snapshot(sources).await)
    }

    async fn poll(
        &self,
        sources: &[ObservedSource<Self::Source>],
        state: &mut Self::State,
    ) -> Result<Vec<Self::Event>, DynError> {
        *state = capture_cluster_snapshot(sources).await;

        Ok(Vec::new())
    }

    fn snapshot(&self, state: &Self::State) -> Self::Snapshot {
        state.clone()
    }
}

/// Builds the fixed source provider used by the scenario-based OpenRaft
/// examples.
pub fn openraft_cluster_source_provider(
    _deployment: &<OpenRaftKvEnv as Application>::Deployment,
    node_clients: NodeClients<OpenRaftKvEnv>,
) -> Result<BoxedSourceProvider<OpenRaftKvClient>, DynError> {
    Ok(Box::new(StaticSourceProvider::new(named_sources(
        node_clients.snapshot(),
    ))))
}

fn named_sources(clients: Vec<OpenRaftKvClient>) -> Vec<ObservedSource<OpenRaftKvClient>> {
    clients
        .into_iter()
        .enumerate()
        .map(|(index, client)| ObservedSource::new(&format!("node-{index}"), client))
        .collect()
}

async fn capture_cluster_snapshot(
    sources: &[ObservedSource<OpenRaftKvClient>],
) -> OpenRaftClusterSnapshot {
    let mut states = Vec::with_capacity(sources.len());
    let mut failures = Vec::new();

    for source in sources {
        match source.source.state().await {
            Ok(state) => states.push(state),
            Err(error) => {
                failures.push(OpenRaftSourceFailure::new(&source.name, &error.to_string()))
            }
        }
    }

    states.sort_by_key(|state| state.node_id);

    OpenRaftClusterSnapshot { states, failures }
}
