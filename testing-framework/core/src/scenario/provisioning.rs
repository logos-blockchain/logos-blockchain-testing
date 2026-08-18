use std::sync::Arc;

use async_trait::async_trait;

use super::{
    Application, ClusterControlProfile, ClusterWaitHandle, DeploymentPolicy, DynError,
    ExistingCluster, ExternalNodeSource, NodeClients, NodeControlHandle, StartNodeOptions,
    StartedNode, internal::CleanupGuard,
};
use crate::topology::DeploymentDescriptor;

/// Source used to provision or connect to one cluster unit.
pub enum ClusterSource<E: Application> {
    Managed {
        deployment: E::Deployment,
        external: Vec<ExternalNodeSource>,
    },
    Attached {
        cluster: ExistingCluster,
        external: Vec<ExternalNodeSource>,
    },
    External {
        nodes: Vec<ExternalNodeSource>,
    },
}

impl<E: Application> Clone for ClusterSource<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Managed {
                deployment,
                external,
            } => Self::Managed {
                deployment: deployment.clone(),
                external: external.clone(),
            },
            Self::Attached { cluster, external } => Self::Attached {
                cluster: cluster.clone(),
                external: external.clone(),
            },
            Self::External { nodes } => Self::External {
                nodes: nodes.clone(),
            },
        }
    }
}

/// Determines whether managed nodes start during provisioning or on demand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterStartMode {
    Eager,
    OnDemand,
}

/// Runtime control requested by the provisioning consumer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterControlRequest {
    None,
    Full,
}

/// Backend-independent request for one cluster unit.
pub struct ClusterRequest<E: Application> {
    source: ClusterSource<E>,
    policy: DeploymentPolicy,
    start_mode: ClusterStartMode,
    control: ClusterControlRequest,
}

impl<E: Application> Clone for ClusterRequest<E> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            policy: self.policy,
            start_mode: self.start_mode,
            control: self.control,
        }
    }
}

impl<E: Application> ClusterRequest<E> {
    #[must_use]
    pub fn managed(deployment: E::Deployment) -> Self {
        Self {
            source: ClusterSource::Managed {
                deployment,
                external: Vec::new(),
            },
            policy: DeploymentPolicy::default(),
            start_mode: ClusterStartMode::Eager,
            control: ClusterControlRequest::None,
        }
    }

    #[must_use]
    pub fn attached(cluster: ExistingCluster) -> Self {
        Self {
            source: ClusterSource::Attached {
                cluster,
                external: Vec::new(),
            },
            policy: DeploymentPolicy::default(),
            start_mode: ClusterStartMode::Eager,
            control: ClusterControlRequest::None,
        }
    }

    #[must_use]
    pub fn external(nodes: Vec<ExternalNodeSource>) -> Self {
        Self {
            source: ClusterSource::External { nodes },
            policy: DeploymentPolicy::default(),
            start_mode: ClusterStartMode::Eager,
            control: ClusterControlRequest::None,
        }
    }

    #[must_use]
    pub fn with_external_nodes(mut self, nodes: Vec<ExternalNodeSource>) -> Self {
        match &mut self.source {
            ClusterSource::Managed { external, .. } | ClusterSource::Attached { external, .. } => {
                external.extend(nodes)
            }
            ClusterSource::External { nodes: external } => external.extend(nodes),
        }
        self
    }

    #[must_use]
    pub const fn with_policy(mut self, policy: DeploymentPolicy) -> Self {
        self.policy = policy;
        self
    }

    #[must_use]
    pub const fn with_start_mode(mut self, start_mode: ClusterStartMode) -> Self {
        self.start_mode = start_mode;
        self
    }

    #[must_use]
    pub const fn with_control(mut self, control: ClusterControlRequest) -> Self {
        self.control = control;
        self
    }

    #[must_use]
    pub const fn source(&self) -> &ClusterSource<E> {
        &self.source
    }

    #[must_use]
    pub const fn policy(&self) -> DeploymentPolicy {
        self.policy
    }

    #[must_use]
    pub const fn start_mode(&self) -> ClusterStartMode {
        self.start_mode
    }

    #[must_use]
    pub const fn control(&self) -> ClusterControlRequest {
        self.control
    }
}

/// Runtime surfaces and lifetime returned for one provisioned cluster.
pub struct ClusterUnit<E: Application> {
    deployment: Option<E::Deployment>,
    node_clients: NodeClients<E>,
    control_profile: ClusterControlProfile,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    cluster_wait: Option<Arc<dyn ClusterWaitHandle<E>>>,
    cleanup: Option<Box<dyn CleanupGuard>>,
}

/// Backend-independent access to one managed, attached, or external cluster.
pub struct ClusterHandle<E: Application> {
    deployment: Option<E::Deployment>,
    node_clients: NodeClients<E>,
    control_profile: ClusterControlProfile,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    cluster_wait: Option<Arc<dyn ClusterWaitHandle<E>>>,
}

impl<E: Application> Clone for ClusterHandle<E> {
    fn clone(&self) -> Self {
        Self {
            deployment: self.deployment.clone(),
            node_clients: self.node_clients.clone(),
            control_profile: self.control_profile,
            node_control: self.node_control.clone(),
            cluster_wait: self.cluster_wait.clone(),
        }
    }
}

impl<E: Application> ClusterHandle<E> {
    #[must_use]
    pub fn deployment(&self) -> Option<&E::Deployment> {
        self.deployment.as_ref()
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.deployment
            .as_ref()
            .map_or(0, DeploymentDescriptor::node_count)
    }

    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        self.node_clients.clone()
    }

    #[must_use]
    pub fn clients(&self) -> Vec<E::NodeClient> {
        self.node_clients.snapshot()
    }

    #[must_use]
    pub fn first_client(&self) -> Option<E::NodeClient> {
        self.node_clients
            .with_clients(|clients| clients.first().cloned())
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.node_control.as_ref()?.node_client(name)
    }

    #[must_use]
    pub fn node_pid(&self, name: &str) -> Option<u32> {
        self.node_control.as_ref()?.node_pid(name)
    }

    #[must_use]
    pub const fn control_profile(&self) -> ClusterControlProfile {
        self.control_profile
    }

    pub async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.require_control()?.start_node(name).await
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        self.require_control()?.start_node_with(name, options).await
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.require_control()?.stop_node(name).await
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.require_control()?.restart_node(name).await
    }

    pub async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        self.require_control()?
            .restart_node_with(name, options)
            .await
    }

    pub async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.cluster_wait
            .as_ref()
            .ok_or_else(|| -> DynError { "cluster readiness is not available".into() })?
            .wait_network_ready()
            .await
    }

    pub async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        self.require_control()?.wait_node_ready(name).await
    }

    fn require_control(&self) -> Result<&Arc<dyn NodeControlHandle<E>>, DynError> {
        self.node_control
            .as_ref()
            .ok_or_else(|| "cluster node control is not available".into())
    }
}

/// Concrete backend handle paired with common runtime surfaces and ownership.
pub struct ProvisionedCluster<E: Application, H> {
    handle: Option<H>,
    unit: ClusterUnit<E>,
}

impl<E: Application, H> ProvisionedCluster<E, H> {
    #[must_use]
    pub const fn new(handle: Option<H>, unit: ClusterUnit<E>) -> Self {
        Self { handle, unit }
    }

    #[must_use]
    pub fn handle(&self) -> Option<&H> {
        self.handle.as_ref()
    }

    #[must_use]
    pub fn into_parts(self) -> (Option<H>, ClusterUnit<E>) {
        (self.handle, self.unit)
    }
}

/// Backend operation used by app deployment contexts to provision a cluster.
#[async_trait]
pub trait ClusterProvisioner<E: Application>: Clone + Send + Sync + 'static {
    async fn provision_cluster(
        &self,
        request: ClusterRequest<E>,
    ) -> Result<ClusterUnit<E>, DynError>;
}

impl<E: Application> ClusterUnit<E> {
    #[must_use]
    pub fn new(
        deployment: Option<E::Deployment>,
        node_clients: NodeClients<E>,
        control_profile: ClusterControlProfile,
    ) -> Self {
        Self {
            deployment,
            node_clients,
            control_profile,
            node_control: None,
            cluster_wait: None,
            cleanup: None,
        }
    }

    #[must_use]
    pub fn with_node_control(mut self, node_control: Arc<dyn NodeControlHandle<E>>) -> Self {
        self.node_control = Some(node_control);
        self
    }

    #[must_use]
    pub fn with_cluster_wait(mut self, cluster_wait: Arc<dyn ClusterWaitHandle<E>>) -> Self {
        self.cluster_wait = Some(cluster_wait);
        self
    }

    #[must_use]
    pub fn with_cleanup(mut self, cleanup: Box<dyn CleanupGuard>) -> Self {
        self.cleanup = Some(cleanup);
        self
    }

    #[must_use]
    pub fn deployment(&self) -> Option<&E::Deployment> {
        self.deployment.as_ref()
    }

    #[must_use]
    pub const fn node_clients(&self) -> &NodeClients<E> {
        &self.node_clients
    }

    #[must_use]
    pub const fn control_profile(&self) -> ClusterControlProfile {
        self.control_profile
    }

    #[must_use]
    pub fn node_control(&self) -> Option<Arc<dyn NodeControlHandle<E>>> {
        self.node_control.clone()
    }

    #[must_use]
    pub fn cluster_wait(&self) -> Option<Arc<dyn ClusterWaitHandle<E>>> {
        self.cluster_wait.clone()
    }

    pub fn take_cleanup(&mut self) -> Option<Box<dyn CleanupGuard>> {
        self.cleanup.take()
    }

    #[must_use]
    pub fn handle(&self) -> ClusterHandle<E> {
        ClusterHandle {
            deployment: self.deployment.clone(),
            node_clients: self.node_clients.clone(),
            control_profile: self.control_profile,
            node_control: self.node_control.clone(),
            cluster_wait: self.cluster_wait.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;

    use super::{
        ClusterControlRequest, ClusterRequest, ClusterSource, ClusterStartMode, ClusterUnit,
    };
    use crate::{
        scenario::{
            Application, CleanupPolicy, ClusterControlProfile, DeploymentPolicy,
            ExternalNodeSource, NodeClients, internal::CleanupGuard,
        },
        topology::NodeCountTopology,
    };

    struct TestApp;

    #[async_trait]
    impl Application for TestApp {
        type Deployment = NodeCountTopology;
        type NodeClient = u8;
        type NodeConfig = ();
    }

    struct CountingCleanup(Arc<AtomicUsize>);

    impl CleanupGuard for CountingCleanup {
        fn cleanup(self: Box<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn cluster_request_preserves_source_policy_and_control_options() {
        let policy = DeploymentPolicy {
            cleanup_policy: CleanupPolicy::new(true),
            ..DeploymentPolicy::default()
        };
        let request = ClusterRequest::<TestApp>::managed(NodeCountTopology::new(2))
            .with_external_nodes(vec![external_node("external-0")])
            .with_policy(policy)
            .with_start_mode(ClusterStartMode::OnDemand)
            .with_control(ClusterControlRequest::Full);

        let ClusterSource::Managed {
            deployment,
            external,
        } = request.source()
        else {
            panic!("managed request changed source kind");
        };

        assert_eq!(deployment.node_count, 2);
        assert_eq!(external.len(), 1);
        assert_eq!(request.policy(), policy);
        assert_eq!(request.start_mode(), ClusterStartMode::OnDemand);
        assert_eq!(request.control(), ClusterControlRequest::Full);
    }

    #[test]
    fn cluster_handle_observes_live_client_inventory() {
        let clients = NodeClients::<TestApp>::new(vec![1, 2]);
        let unit = ClusterUnit::new(
            Some(NodeCountTopology::new(2)),
            clients.clone(),
            ClusterControlProfile::FrameworkManaged,
        );
        let handle = unit.handle();

        clients.add_node(3);

        assert_eq!(handle.node_count(), 2);
        assert_eq!(handle.clients(), vec![1, 2, 3]);
        assert_eq!(handle.first_client(), Some(1));
    }

    #[tokio::test]
    async fn cluster_handle_reports_missing_runtime_capabilities() {
        let unit = ClusterUnit::<TestApp>::new(
            None,
            NodeClients::default(),
            ClusterControlProfile::ExternalUncontrolled,
        );
        let handle = unit.handle();

        assert_eq!(
            handle
                .restart_node("node-0")
                .await
                .expect_err("missing node control must fail")
                .to_string(),
            "cluster node control is not available"
        );
        assert_eq!(
            handle
                .wait_network_ready()
                .await
                .expect_err("missing readiness handle must fail")
                .to_string(),
            "cluster readiness is not available"
        );
    }

    #[test]
    fn cluster_cleanup_can_only_be_taken_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut unit = ClusterUnit::<TestApp>::new(
            None,
            NodeClients::default(),
            ClusterControlProfile::ExternalUncontrolled,
        )
        .with_cleanup(Box::new(CountingCleanup(Arc::clone(&calls))));

        unit.take_cleanup().expect("cleanup guard").cleanup();

        assert!(unit.take_cleanup().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn external_node(name: &str) -> ExternalNodeSource {
        ExternalNodeSource::new(name.to_owned(), "http://127.0.0.1:1".to_owned())
    }
}
