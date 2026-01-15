use std::{
    pin::Pin,
    sync::{Arc, RwLock},
};

use rand::{Rng as _, seq::SliceRandom as _, thread_rng};

use crate::{
    nodes::ApiClient,
    scenario::DynError,
    topology::{deployment::Topology, generation::GeneratedTopology},
};

/// Collection of API clients for the validator and executor set.
#[derive(Clone, Default)]
pub struct NodeClients {
    inner: Arc<RwLock<NodeClientsInner>>,
}

#[derive(Default)]
struct NodeClientsInner {
    validators: Vec<ApiClient>,
    executors: Vec<ApiClient>,
}

impl NodeClients {
    #[must_use]
    /// Build clients from preconstructed vectors.
    pub fn new(validators: Vec<ApiClient>, executors: Vec<ApiClient>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(NodeClientsInner {
                validators,
                executors,
            })),
        }
    }

    #[must_use]
    /// Derive clients from a spawned topology.
    pub fn from_topology(_descriptors: &GeneratedTopology, topology: &Topology) -> Self {
        let validator_clients = topology.validators().iter().map(|node| {
            let testing = node.testing_url();
            ApiClient::from_urls(node.url(), testing)
        });

        let executor_clients = topology.executors().iter().map(|node| {
            let testing = node.testing_url();
            ApiClient::from_urls(node.url(), testing)
        });

        Self::new(validator_clients.collect(), executor_clients.collect())
    }

    #[must_use]
    /// Validator API clients.
    pub fn validator_clients(&self) -> Vec<ApiClient> {
        self.inner
            .read()
            .expect("node clients lock poisoned")
            .validators
            .clone()
    }

    #[must_use]
    /// Executor API clients.
    pub fn executor_clients(&self) -> Vec<ApiClient> {
        self.inner
            .read()
            .expect("node clients lock poisoned")
            .executors
            .clone()
    }

    #[must_use]
    /// Choose a random validator client if present.
    pub fn random_validator(&self) -> Option<ApiClient> {
        let validators = self.validator_clients();
        if validators.is_empty() {
            return None;
        }
        let mut rng = thread_rng();
        let idx = rng.gen_range(0..validators.len());
        validators.get(idx).cloned()
    }

    #[must_use]
    /// Choose a random executor client if present.
    pub fn random_executor(&self) -> Option<ApiClient> {
        let executors = self.executor_clients();
        if executors.is_empty() {
            return None;
        }
        let mut rng = thread_rng();
        let idx = rng.gen_range(0..executors.len());
        executors.get(idx).cloned()
    }

    /// Iterator over all clients.
    pub fn all_clients(&self) -> Vec<ApiClient> {
        let guard = self.inner.read().expect("node clients lock poisoned");
        guard
            .validators
            .iter()
            .chain(guard.executors.iter())
            .cloned()
            .collect()
    }

    #[must_use]
    /// Choose any random client from validators+executors.
    pub fn any_client(&self) -> Option<ApiClient> {
        let guard = self.inner.read().expect("node clients lock poisoned");
        let validator_count = guard.validators.len();
        let executor_count = guard.executors.len();
        let total = validator_count + executor_count;
        if total == 0 {
            return None;
        }
        let mut rng = thread_rng();
        let choice = rng.gen_range(0..total);
        if choice < validator_count {
            guard.validators.get(choice).cloned()
        } else {
            guard.executors.get(choice - validator_count).cloned()
        }
    }

    #[must_use]
    /// Convenience wrapper for fan-out queries.
    pub const fn cluster_client(&self) -> ClusterClient<'_> {
        ClusterClient::new(self)
    }

    pub fn add_validator(&self, client: ApiClient) {
        let mut guard = self.inner.write().expect("node clients lock poisoned");
        guard.validators.push(client);
    }

    pub fn add_executor(&self, client: ApiClient) {
        let mut guard = self.inner.write().expect("node clients lock poisoned");
        guard.executors.push(client);
    }
}

pub struct ClusterClient<'a> {
    node_clients: &'a NodeClients,
}

impl<'a> ClusterClient<'a> {
    #[must_use]
    /// Build a cluster client that can try multiple nodes.
    pub const fn new(node_clients: &'a NodeClients) -> Self {
        Self { node_clients }
    }

    /// Try all node clients until one call succeeds, shuffling order each time.
    pub async fn try_all_clients<T, E>(
        &self,
        mut f: impl for<'b> FnMut(
            &'b ApiClient,
        ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'b>>
        + Send,
    ) -> Result<T, DynError>
    where
        E: Into<DynError>,
    {
        let mut clients = self.node_clients.all_clients();
        if clients.is_empty() {
            return Err("cluster client has no api clients".into());
        }

        clients.shuffle(&mut thread_rng());

        let mut last_err = None;
        for client in &clients {
            match f(client).await {
                Ok(value) => return Ok(value),
                Err(err) => last_err = Some(err.into()),
            }
        }

        Err(last_err.unwrap_or_else(|| "cluster client exhausted all nodes".into()))
    }
}
