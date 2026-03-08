use rand::{seq::SliceRandom as _, thread_rng};

use super::inventory::NodeInventory;
use crate::scenario::{Application, DynError};

/// Collection of API clients for the node set.
pub struct NodeClients<E: Application> {
    inventory: NodeInventory<E>,
}

impl<E: Application> Default for NodeClients<E> {
    fn default() -> Self {
        Self {
            inventory: NodeInventory::default(),
        }
    }
}

impl<E: Application> Clone for NodeClients<E> {
    fn clone(&self) -> Self {
        Self {
            inventory: self.inventory.clone(),
        }
    }
}

impl<E: Application> NodeClients<E> {
    #[must_use]
    /// Build clients from preconstructed vectors.
    pub fn new(nodes: Vec<E::NodeClient>) -> Self {
        Self {
            inventory: NodeInventory::from_managed_clients(nodes),
        }
    }

    #[must_use]
    /// Immutable client snapshot at the current moment.
    ///
    /// This clones the current vector so callers can iterate across `.await`
    /// boundaries without holding the internal lock.
    pub fn snapshot(&self) -> Vec<E::NodeClient> {
        self.inventory.snapshot_clients()
    }

    /// Execute a synchronous read against the current client slice.
    ///
    /// Prefer this over `snapshot()` when no async boundary is involved.
    pub fn with_clients<R>(&self, f: impl FnOnce(&[E::NodeClient]) -> R) -> R {
        self.inventory.with_clients(f)
    }

    #[must_use]
    /// Choose a random client from the current snapshot.
    pub fn random_client(&self) -> Option<E::NodeClient> {
        self.with_clients(|clients| clients.choose(&mut thread_rng()).cloned())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inventory.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inventory.is_empty()
    }

    #[must_use]
    /// Convenience wrapper for fan-out queries.
    pub const fn cluster_client(&self) -> ClusterClient<'_, E> {
        ClusterClient::new(self)
    }

    pub fn add_node(&self, client: E::NodeClient) {
        self.inventory.add_managed_node(client, None);
    }

    pub fn clear(&self) {
        self.inventory.clear();
    }

    fn shuffled_snapshot(&self) -> Vec<E::NodeClient> {
        let mut clients = self.snapshot();
        clients.shuffle(&mut thread_rng());
        clients
    }
}

pub struct ClusterClient<'a, E: Application> {
    node_clients: &'a NodeClients<E>,
}

impl<'a, E: Application> ClusterClient<'a, E> {
    #[must_use]
    pub const fn new(node_clients: &'a NodeClients<E>) -> Self {
        Self { node_clients }
    }

    pub async fn try_all_clients<Value, ErrorType, Fut>(
        &self,
        mut f: impl for<'b> FnMut(&'b E::NodeClient) -> Fut + Send,
    ) -> Result<Value, DynError>
    where
        for<'b> Fut: Future<Output = Result<Value, ErrorType>> + Send + 'b,
        ErrorType: Into<DynError>,
    {
        // Snapshot once so retries can await without holding the internal lock.
        let clients = self.node_clients.shuffled_snapshot();

        if clients.is_empty() {
            return Err("cluster client has no api clients".into());
        }

        let mut last_error = None;
        for client in &clients {
            match f(client).await {
                Ok(value) => return Ok(value),
                Err(error) => last_error = Some(error.into()),
            }
        }

        if let Some(error) = last_error {
            return Err(error);
        }

        Err("cluster client exhausted all nodes".into())
    }
}
