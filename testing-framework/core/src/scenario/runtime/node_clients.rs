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

/// Collection of API clients for the validatorset.
#[derive(Clone, Default)]
pub struct NodeClients {
    inner: Arc<RwLock<NodeClientsInner>>,
}

#[derive(Default)]
struct NodeClientsInner {
    validators: Vec<ApiClient>,
}

impl NodeClients {
    #[must_use]
    /// Build clients from preconstructed vectors.
    pub fn new(validators: Vec<ApiClient>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(NodeClientsInner { validators })),
        }
    }

    #[must_use]
    /// Derive clients from a spawned topology.
    pub fn from_topology(_descriptors: &GeneratedTopology, topology: &Topology) -> Self {
        let validator_clients = topology.validators().iter().map(|node| {
            let testing = node.testing_url();
            ApiClient::from_urls(node.url(), testing)
        });

        Self::new(validator_clients.collect())
    }

    #[must_use]
    /// Validator API clients.
    pub fn validator_clients(&self) -> Vec<ApiClient> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .validators
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

    /// Iterator over all clients.
    pub fn all_clients(&self) -> Vec<ApiClient> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        guard.validators.iter().cloned().collect()
    }

    #[must_use]
    /// Choose any random client from validators.
    pub fn any_client(&self) -> Option<ApiClient> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let validator_count = guard.validators.len();
        let total = validator_count;
        if total == 0 {
            return None;
        }
        let mut rng = thread_rng();
        let choice = rng.gen_range(0..total);
        guard.validators.get(choice).cloned()
    }

    #[must_use]
    /// Convenience wrapper for fan-out queries.
    pub const fn cluster_client(&self) -> ClusterClient<'_> {
        ClusterClient::new(self)
    }

    pub fn add_validator(&self, client: ApiClient) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        guard.validators.push(client);
    }

    pub fn clear(&self) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        guard.validators.clear();
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
