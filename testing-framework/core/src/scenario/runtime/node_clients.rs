use std::pin::Pin;

use rand::{Rng as _, seq::SliceRandom as _, thread_rng};

use crate::{
    nodes::ApiClient,
    scenario::DynError,
    topology::{deployment::Topology, generation::GeneratedTopology},
};

/// Collection of API clients for the validator and executor set.
#[derive(Clone, Default)]
pub struct NodeClients {
    validators: Vec<ApiClient>,
    executors: Vec<ApiClient>,
}

impl NodeClients {
    #[must_use]
    /// Build clients from preconstructed vectors.
    pub const fn new(validators: Vec<ApiClient>, executors: Vec<ApiClient>) -> Self {
        Self {
            validators,
            executors,
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
    pub fn validator_clients(&self) -> &[ApiClient] {
        &self.validators
    }

    #[must_use]
    /// Executor API clients.
    pub fn executor_clients(&self) -> &[ApiClient] {
        &self.executors
    }

    #[must_use]
    /// Choose a random validator client if present.
    pub fn random_validator(&self) -> Option<&ApiClient> {
        if self.validators.is_empty() {
            return None;
        }
        let mut rng = thread_rng();
        let idx = rng.gen_range(0..self.validators.len());
        self.validators.get(idx)
    }

    #[must_use]
    /// Choose a random executor client if present.
    pub fn random_executor(&self) -> Option<&ApiClient> {
        if self.executors.is_empty() {
            return None;
        }
        let mut rng = thread_rng();
        let idx = rng.gen_range(0..self.executors.len());
        self.executors.get(idx)
    }

    /// Iterator over all clients.
    pub fn all_clients(&self) -> impl Iterator<Item = &ApiClient> {
        self.validators.iter().chain(self.executors.iter())
    }

    #[must_use]
    /// Choose any random client from validators+executors.
    pub fn any_client(&self) -> Option<&ApiClient> {
        let validator_count = self.validators.len();
        let executor_count = self.executors.len();
        let total = validator_count + executor_count;
        if total == 0 {
            return None;
        }
        let mut rng = thread_rng();
        let choice = rng.gen_range(0..total);
        if choice < validator_count {
            self.validators.get(choice)
        } else {
            self.executors.get(choice - validator_count)
        }
    }

    #[must_use]
    /// Convenience wrapper for fan-out queries.
    pub const fn cluster_client(&self) -> ClusterClient<'_> {
        ClusterClient::new(self)
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
        let mut clients: Vec<&ApiClient> = self.node_clients.all_clients().collect();
        if clients.is_empty() {
            return Err("cluster client has no api clients".into());
        }

        clients.shuffle(&mut thread_rng());

        let mut last_err = None;
        for client in clients {
            match f(client).await {
                Ok(value) => return Ok(value),
                Err(err) => last_err = Some(err.into()),
            }
        }

        Err(last_err.unwrap_or_else(|| "cluster client exhausted all nodes".into()))
    }
}
