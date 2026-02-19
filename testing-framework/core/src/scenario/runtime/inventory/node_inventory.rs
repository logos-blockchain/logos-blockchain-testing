use std::{collections::HashMap, sync::Arc};

use parking_lot::RwLock;

use crate::scenario::{Application, DynError, NodeControlHandle, StartNodeOptions, StartedNode};

/// Origin for borrowed (non-managed) nodes in the runtime inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BorrowedOrigin {
    /// Node discovered from an attached cluster provider.
    Attached,
    /// Node provided explicitly as an external endpoint.
    External,
}

/// Managed node handle with full lifecycle capabilities.
pub struct ManagedNode<E: Application> {
    /// Canonical node identity used for deduplication and lookups.
    pub identity: String,
    /// Application-specific API client for this node.
    pub client: E::NodeClient,
}

/// Borrowed node handle (attached or external), query-only by default.
pub struct BorrowedNode<E: Application> {
    /// Canonical node identity used for deduplication and lookups.
    pub identity: String,
    /// Application-specific API client for this node.
    pub client: E::NodeClient,
    /// Borrowed source kind used for diagnostics and selection.
    pub origin: BorrowedOrigin,
}

/// Unified node handle variant used by runtime inventory snapshots.
pub enum NodeHandle<E: Application> {
    /// Managed node variant.
    Managed(ManagedNode<E>),
    /// Borrowed node variant.
    Borrowed(BorrowedNode<E>),
}

impl<E: Application> Clone for ManagedNode<E> {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            client: self.client.clone(),
        }
    }
}

impl<E: Application> Clone for BorrowedNode<E> {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            client: self.client.clone(),
            origin: self.origin,
        }
    }
}

impl<E: Application> Clone for NodeHandle<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Managed(node) => Self::Managed(node.clone()),
            Self::Borrowed(node) => Self::Borrowed(node.clone()),
        }
    }
}

/// Thread-safe node inventory with identity-based upsert semantics.
pub struct NodeInventory<E: Application> {
    inner: Arc<RwLock<NodeInventoryInner<E>>>,
}

struct NodeInventoryInner<E: Application> {
    nodes: Vec<NodeHandle<E>>,
    indices_by_identity: HashMap<String, usize>,
    next_synthetic_id: usize,
}

impl<E: Application> Default for NodeInventory<E> {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(NodeInventoryInner {
                nodes: Vec::new(),
                indices_by_identity: HashMap::new(),
                next_synthetic_id: 0,
            })),
        }
    }
}

impl<E: Application> Clone for NodeInventory<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<E: Application> NodeInventory<E> {
    #[must_use]
    /// Builds an inventory from managed clients.
    pub fn from_managed_clients(clients: Vec<E::NodeClient>) -> Self {
        let inventory = Self::default();

        for client in clients {
            inventory.add_managed_node(client, None);
        }

        inventory
    }

    #[must_use]
    /// Returns a cloned snapshot of all node clients.
    pub fn snapshot_clients(&self) -> Vec<E::NodeClient> {
        self.inner.read().nodes.iter().map(clone_client).collect()
    }

    #[must_use]
    /// Returns cloned managed node handles from the current inventory.
    pub fn managed_nodes(&self) -> Vec<ManagedNode<E>> {
        self.inner
            .read()
            .nodes
            .iter()
            .filter_map(|handle| match handle {
                NodeHandle::Managed(node) => Some(node.clone()),
                NodeHandle::Borrowed(_) => None,
            })
            .collect()
    }

    #[must_use]
    /// Returns cloned borrowed node handles from the current inventory.
    pub fn borrowed_nodes(&self) -> Vec<BorrowedNode<E>> {
        self.inner
            .read()
            .nodes
            .iter()
            .filter_map(|handle| match handle {
                NodeHandle::Managed(_) => None,
                NodeHandle::Borrowed(node) => Some(node.clone()),
            })
            .collect()
    }

    #[must_use]
    /// Finds a managed node by canonical identity.
    pub fn find_managed(&self, identity: &str) -> Option<ManagedNode<E>> {
        let guard = self.inner.read();
        match node_by_identity(&guard, identity)? {
            NodeHandle::Managed(node) => Some(node.clone()),
            NodeHandle::Borrowed(_) => None,
        }
    }

    #[must_use]
    /// Finds a borrowed node by canonical identity.
    pub fn find_borrowed(&self, identity: &str) -> Option<BorrowedNode<E>> {
        let guard = self.inner.read();
        match node_by_identity(&guard, identity)? {
            NodeHandle::Managed(_) => None,
            NodeHandle::Borrowed(node) => Some(node.clone()),
        }
    }

    #[must_use]
    /// Finds any node handle by canonical identity.
    pub fn find_node(&self, identity: &str) -> Option<NodeHandle<E>> {
        let guard = self.inner.read();
        node_by_identity(&guard, identity).cloned()
    }

    #[must_use]
    /// Returns current number of nodes in inventory.
    pub fn len(&self) -> usize {
        self.inner.read().nodes.len()
    }

    #[must_use]
    /// Returns true when no nodes are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears all nodes and identity indexes.
    pub fn clear(&self) {
        let mut guard = self.inner.write();
        guard.nodes.clear();
        guard.indices_by_identity.clear();
        guard.next_synthetic_id = 0;
    }

    /// Adds or replaces a managed node entry using canonical identity
    /// resolution. Re-adding the same node identity updates the stored handle.
    pub fn add_managed_node(&self, client: E::NodeClient, identity_hint: Option<String>) {
        let mut guard = self.inner.write();
        let identity = canonical_identity::<E>(&client, identity_hint, &mut guard);
        let handle = NodeHandle::Managed(ManagedNode {
            identity: identity.clone(),
            client,
        });
        upsert_node(&mut guard, identity, handle);
    }

    /// Adds or replaces an attached node entry.
    pub fn add_attached_node(&self, client: E::NodeClient, identity_hint: Option<String>) {
        self.add_borrowed_node(client, BorrowedOrigin::Attached, identity_hint);
    }

    /// Adds or replaces an external static node entry.
    pub fn add_external_node(&self, client: E::NodeClient, identity_hint: Option<String>) {
        self.add_borrowed_node(client, BorrowedOrigin::External, identity_hint);
    }

    /// Executes a synchronous read over a cloned client slice.
    pub fn with_clients<R>(&self, f: impl FnOnce(&[E::NodeClient]) -> R) -> R {
        let guard = self.inner.read();
        let clients = guard.nodes.iter().map(clone_client).collect::<Vec<_>>();
        f(&clients)
    }

    fn add_borrowed_node(
        &self,
        client: E::NodeClient,
        origin: BorrowedOrigin,
        identity_hint: Option<String>,
    ) {
        let mut guard = self.inner.write();
        let identity = canonical_identity::<E>(&client, identity_hint, &mut guard);
        let handle = NodeHandle::Borrowed(BorrowedNode {
            identity: identity.clone(),
            client,
            origin,
        });
        upsert_node(&mut guard, identity, handle);
    }
}

impl<E: Application> ManagedNode<E> {
    #[must_use]
    /// Returns the node client.
    pub const fn client(&self) -> &E::NodeClient {
        &self.client
    }

    /// Delegates restart to the deployer's control surface for this node name.
    pub async fn restart(
        &self,
        control: &dyn NodeControlHandle<E>,
        node_name: &str,
    ) -> Result<(), DynError> {
        control.restart_node(node_name).await
    }

    /// Delegates stop to the deployer's control surface for this node name.
    pub async fn stop(
        &self,
        control: &dyn NodeControlHandle<E>,
        node_name: &str,
    ) -> Result<(), DynError> {
        control.stop_node(node_name).await
    }

    /// Delegates dynamic node start with options to the control surface.
    pub async fn start_with(
        &self,
        control: &dyn NodeControlHandle<E>,
        node_name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        control.start_node_with(node_name, options).await
    }

    #[must_use]
    /// Returns process id if the backend can expose it for this node name.
    pub fn pid(&self, control: &dyn NodeControlHandle<E>, node_name: &str) -> Option<u32> {
        control.node_pid(node_name)
    }
}

impl<E: Application> BorrowedNode<E> {
    #[must_use]
    /// Returns the node client.
    pub const fn client(&self) -> &E::NodeClient {
        &self.client
    }
}

fn upsert_node<E: Application>(
    inner: &mut NodeInventoryInner<E>,
    identity: String,
    handle: NodeHandle<E>,
) {
    if let Some(existing_index) = inner.indices_by_identity.get(&identity).copied() {
        inner.nodes[existing_index] = handle;
        return;
    }

    let index = inner.nodes.len();
    inner.nodes.push(handle);
    inner.indices_by_identity.insert(identity, index);
}

fn canonical_identity<E: Application>(
    _client: &E::NodeClient,
    identity_hint: Option<String>,
    inner: &mut NodeInventoryInner<E>,
) -> String {
    // Priority: explicit hint -> synthetic.
    if let Some(identity) = identity_hint.filter(|value| !value.trim().is_empty()) {
        return identity;
    }

    let synthetic = format!("node:{}", inner.next_synthetic_id);
    inner.next_synthetic_id += 1;

    synthetic
}

fn clone_client<E: Application>(handle: &NodeHandle<E>) -> E::NodeClient {
    match handle {
        NodeHandle::Managed(node) => node.client.clone(),
        NodeHandle::Borrowed(node) => node.client.clone(),
    }
}

fn node_by_identity<'a, E: Application>(
    inner: &'a NodeInventoryInner<E>,
    identity: &str,
) -> Option<&'a NodeHandle<E>> {
    let index = *inner.indices_by_identity.get(identity)?;
    inner.nodes.get(index)
}
