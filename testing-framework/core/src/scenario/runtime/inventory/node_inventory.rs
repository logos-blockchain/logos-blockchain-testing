use std::sync::Arc;

use parking_lot::RwLock;

use crate::scenario::Application;

/// Thread-safe node client storage used by runtime handles.
pub(crate) struct NodeInventory<E: Application> {
    clients: Arc<RwLock<Vec<E::NodeClient>>>,
}

impl<E: Application> Default for NodeInventory<E> {
    fn default() -> Self {
        Self {
            clients: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl<E: Application> Clone for NodeInventory<E> {
    fn clone(&self) -> Self {
        Self {
            clients: Arc::clone(&self.clients),
        }
    }
}

impl<E: Application> NodeInventory<E> {
    #[must_use]
    pub(crate) fn from_clients(clients: Vec<E::NodeClient>) -> Self {
        Self {
            clients: Arc::new(RwLock::new(clients)),
        }
    }

    #[must_use]
    pub(crate) fn snapshot_clients(&self) -> Vec<E::NodeClient> {
        self.clients.read().clone()
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.clients.read().len()
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn clear(&self) {
        self.clients.write().clear();
    }

    pub(crate) fn add_client(&self, client: E::NodeClient) {
        self.clients.write().push(client);
    }

    pub(crate) fn with_clients<R>(&self, f: impl FnOnce(&[E::NodeClient]) -> R) -> R {
        let clients = self.clients.read();
        f(&clients)
    }
}
