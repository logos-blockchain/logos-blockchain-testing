use testing_framework_core::scenario::{
    Application, ClusterControlRequest, ClusterHandle, ClusterProvisioner, ClusterRequest,
    DynError, NodeClients, internal::CleanupGuard,
};
use testing_framework_runner_local::{LocalClusterProvisioner, LocalDeployerEnv};

use crate::{
    AppDeployError, AppDeployment, AppHandle, HandleRegistry, InlineAppDeployment, LocalAppCluster,
    cleanup::AppCleanupStack,
};

/// Mutable deployment context used to compose applications and expose handles.
///
/// A context belongs to one scenario preparation. It provides access to the
/// outer scenario deployment and node clients while collecting application
/// handles for later use by workloads. Exposed handles are dropped in reverse
/// exposure order when the scenario runtime is released.
pub struct DeployContext<E: Application, P = LocalClusterProvisioner> {
    deployment: E::Deployment,
    node_clients: NodeClients<E>,
    provisioner: P,
    handles: HandleRegistry,
    cleanup: AppCleanupStack,
}

impl<E> DeployContext<E, LocalClusterProvisioner>
where
    E: Application,
{
    /// Creates a context for an outer scenario deployment and its node clients.
    pub fn new(deployment: E::Deployment, node_clients: NodeClients<E>) -> Self {
        Self::new_with_provisioner(deployment, node_clients, LocalClusterProvisioner)
    }
}

impl<E, P> DeployContext<E, P>
where
    E: Application,
    P: Send + Sync + 'static,
{
    /// Creates a context backed by an explicit cluster provisioner.
    pub fn new_with_provisioner(
        deployment: E::Deployment,
        node_clients: NodeClients<E>,
        provisioner: P,
    ) -> Self {
        Self {
            deployment,
            node_clients,
            provisioner,
            handles: HandleRegistry::new(),
            cleanup: AppCleanupStack::default(),
        }
    }

    /// Deploys a child application without automatically exposing its handle.
    ///
    /// Use this when the returned handle is only needed while constructing a
    /// higher-level application handle.
    pub async fn deploy<A>(&mut self, app: A) -> Result<A::Handle, DynError>
    where
        A: AppDeployment<E, P>,
    {
        app.deploy(self).await
    }

    /// Deploys a child application and exposes a clone of its handle.
    ///
    /// The exposed clone makes the handle available to workloads through
    /// [`crate::AppRunContextExt`]. Managed resource lifetime remains owned by
    /// scenario cleanup.
    pub async fn deploy_and_expose<A>(&mut self, app: A) -> Result<A::Handle, DynError>
    where
        A: AppDeployment<E, P>,
    {
        let handle = self.deploy(app).await?;

        self.expose(handle.clone())?;

        Ok(handle)
    }

    /// Deploys a child application inline without automatically exposing its
    /// handle.
    ///
    /// This is the non-`Send` counterpart to [`Self::deploy`]. It is intended
    /// for an explicitly caller-owned local async path, such as a Cucumber
    /// world or another in-process harness.
    pub async fn deploy_inline<A>(&mut self, app: A) -> Result<A::Handle, DynError>
    where
        A: InlineAppDeployment<E, P>,
    {
        app.deploy_inline(self).await
    }

    /// Deploys a child application inline and exposes a clone of its handle.
    pub async fn deploy_and_expose_inline<A>(&mut self, app: A) -> Result<A::Handle, DynError>
    where
        A: InlineAppDeployment<E, P>,
    {
        let handle = self.deploy_inline(app).await?;

        self.expose(handle.clone())?;

        Ok(handle)
    }

    /// Exposes the default unnamed handle for its concrete type.
    ///
    /// Returns [`AppDeployError::DuplicateHandle`] when that type already has a
    /// default handle.
    pub fn expose<T>(&mut self, handle: T) -> Result<(), AppDeployError>
    where
        T: AppHandle,
    {
        self.handles.expose(handle)
    }

    /// Exposes a named handle, allowing multiple instances of the same type.
    ///
    /// A type and name pair must be unique within the deployment context.
    pub fn expose_named<T>(
        &mut self,
        name: impl Into<String>,
        handle: T,
    ) -> Result<(), AppDeployError>
    where
        T: AppHandle,
    {
        self.handles.expose_named(name, handle)
    }

    /// Returns a clone of the default handle for `T`, if it is exposed.
    pub fn get<T>(&self) -> Option<T>
    where
        T: AppHandle,
    {
        self.handles.get()
    }

    /// Returns a clone of the named handle for `T`, if it is exposed.
    pub fn get_named<T>(&self, name: &str) -> Option<T>
    where
        T: AppHandle,
    {
        self.handles.get_named(name)
    }

    /// Returns the default handle for `T` or a typed missing-handle error.
    pub fn require<T>(&self) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.handles.require()
    }

    /// Returns the named handle for `T` or a typed missing-handle error.
    pub fn require_named<T>(&self, name: &str) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.handles.require_named(name)
    }

    /// Returns whether the default handle for `T` is exposed.
    pub fn contains<T>(&self) -> bool
    where
        T: AppHandle,
    {
        self.handles.contains::<T>()
    }

    /// Borrows the handles exposed so far.
    pub fn handles(&self) -> &HandleRegistry {
        &self.handles
    }

    /// Provisions a cluster through the active local backend.
    pub async fn deploy_cluster<App>(
        &mut self,
        request: ClusterRequest<App>,
    ) -> Result<ClusterHandle<App>, DynError>
    where
        App: Application,
        P: ClusterProvisioner<App>,
    {
        let mut unit = self
            .provisioner
            .provision_cluster(request.with_control(ClusterControlRequest::Full))
            .await?;
        let handle = unit.handle();
        if let Some(cleanup) = unit.take_cleanup() {
            self.register_cleanup(cleanup);
        }
        Ok(handle)
    }

    /// Starts every node of an additional uniform local cluster.
    ///
    /// The cluster is registered with scenario cleanup before its handle is
    /// returned. Cleanup stops all nodes independently of handle clones.
    pub async fn deploy_local_cluster<App>(
        &mut self,
        deployment: App::Deployment,
    ) -> Result<LocalAppCluster<App>, DynError>
    where
        App: LocalDeployerEnv,
        P: ClusterProvisioner<App>,
    {
        self.deploy_cluster(ClusterRequest::managed(deployment))
            .await
    }

    /// Returns the outer scenario deployment descriptor.
    pub fn deployment(&self) -> &E::Deployment {
        &self.deployment
    }

    /// Returns the outer scenario clients resolved from all configured sources.
    pub fn node_clients(&self) -> &NodeClients<E> {
        &self.node_clients
    }

    pub(crate) fn register_cleanup(&mut self, guard: Box<dyn CleanupGuard>) {
        self.cleanup.push(guard);
    }

    pub(crate) fn into_runtime_parts(self) -> (HandleRegistry, Option<Box<dyn CleanupGuard>>) {
        let Self {
            handles, cleanup, ..
        } = self;
        (handles, cleanup.into_guard())
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use async_trait::async_trait;
    use testing_framework_core::{
        scenario::{Application, DynError, NodeClients},
        topology::DeploymentDescriptor,
    };

    use super::DeployContext;
    use crate::{AppDeployment, InlineAppDeployment};

    #[derive(Clone)]
    struct TestDeployment;

    impl DeploymentDescriptor for TestDeployment {
        fn node_count(&self) -> usize {
            0
        }
    }

    struct TestEnv;

    #[async_trait]
    impl Application for TestEnv {
        type Deployment = TestDeployment;
        type NodeClient = ();
        type NodeConfig = ();
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct ChildHandle {
        id: &'static str,
    }

    struct ChildApp;

    #[async_trait]
    impl AppDeployment<TestEnv> for ChildApp {
        type Handle = ChildHandle;

        async fn deploy(self, _ctx: &mut DeployContext<TestEnv>) -> Result<Self::Handle, DynError> {
            Ok(ChildHandle { id: "child" })
        }
    }

    struct InlineChildApp;

    #[async_trait(?Send)]
    impl InlineAppDeployment<TestEnv> for InlineChildApp {
        type Handle = ChildHandle;

        async fn deploy_inline(
            self,
            _ctx: &mut DeployContext<TestEnv>,
        ) -> Result<Self::Handle, DynError> {
            let marker = Rc::new("inline");
            tokio::task::yield_now().await;
            assert_eq!(*marker, "inline");

            Ok(ChildHandle { id: "inline-child" })
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct InlineParentHandle {
        child: ChildHandle,
    }

    struct InlineParentApp;

    #[async_trait(?Send)]
    impl InlineAppDeployment<TestEnv> for InlineParentApp {
        type Handle = InlineParentHandle;

        async fn deploy_inline(
            self,
            ctx: &mut DeployContext<TestEnv>,
        ) -> Result<Self::Handle, DynError> {
            let child = ctx.deploy_and_expose_inline(InlineChildApp).await?;
            let parent = InlineParentHandle { child };
            ctx.expose(parent.clone())?;

            Ok(parent)
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct SiblingHandle {
        id: &'static str,
    }

    struct SiblingApp;

    #[async_trait]
    impl AppDeployment<TestEnv> for SiblingApp {
        type Handle = SiblingHandle;

        async fn deploy(self, _ctx: &mut DeployContext<TestEnv>) -> Result<Self::Handle, DynError> {
            Ok(SiblingHandle { id: "sibling" })
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct ParentHandle {
        child: ChildHandle,
    }

    struct ParentApp;

    #[async_trait]
    impl AppDeployment<TestEnv> for ParentApp {
        type Handle = ParentHandle;

        async fn deploy(self, ctx: &mut DeployContext<TestEnv>) -> Result<Self::Handle, DynError> {
            let child = ctx.deploy(ChildApp).await?;
            let parent = ParentHandle { child };

            ctx.expose(parent.clone())?;

            Ok(parent)
        }
    }

    #[tokio::test]
    async fn deploy_returns_handle_without_exposing_it() {
        let mut ctx = test_context();
        let child = ctx.deploy(ChildApp).await.expect("deploy child app");

        assert_eq!(child, ChildHandle { id: "child" });
        assert!(ctx.get::<ChildHandle>().is_none());
    }

    #[tokio::test]
    async fn exposed_handle_can_be_required() {
        let mut ctx = test_context();

        ctx.expose(ChildHandle { id: "child" })
            .expect("expose child");

        let child = ctx.require::<ChildHandle>().expect("require exposed child");
        assert_eq!(child, ChildHandle { id: "child" });
    }

    #[tokio::test]
    async fn nested_deployment_exposes_parent_handle_only() {
        let mut ctx = test_context();
        let parent = ctx.deploy(ParentApp).await.expect("deploy parent app");

        assert_eq!(parent.child, ChildHandle { id: "child" });
        assert!(ctx.get::<ChildHandle>().is_none());
        assert!(ctx.get::<ParentHandle>().is_some());
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct StackHandle {
        child: ChildHandle,
        sibling: SiblingHandle,
    }

    struct StackApp;

    #[async_trait]
    impl AppDeployment<TestEnv> for StackApp {
        type Handle = StackHandle;

        async fn deploy(self, ctx: &mut DeployContext<TestEnv>) -> Result<Self::Handle, DynError> {
            let child = ctx.deploy_and_expose(ChildApp).await?;
            let sibling = ctx.deploy_and_expose(SiblingApp).await?;
            let stack = StackHandle { child, sibling };

            ctx.expose(stack.clone())?;

            Ok(stack)
        }
    }

    #[tokio::test]
    async fn nested_deployment_can_expose_multiple_typed_handles() {
        let mut ctx = test_context();
        let stack = ctx.deploy(StackApp).await.expect("deploy stack app");

        assert_eq!(stack.child, ChildHandle { id: "child" });
        assert_eq!(stack.sibling, SiblingHandle { id: "sibling" });
        assert!(ctx.get::<ChildHandle>().is_some());
        assert!(ctx.get::<SiblingHandle>().is_some());
        assert!(ctx.get::<StackHandle>().is_some());
    }

    #[tokio::test]
    async fn inline_deployment_can_hold_non_send_state_across_an_await() {
        let mut ctx = test_context();
        let child = ctx
            .deploy_inline(InlineChildApp)
            .await
            .expect("deploy inline child app");

        assert_eq!(child, ChildHandle { id: "inline-child" });
        assert!(ctx.get::<ChildHandle>().is_none());
    }

    #[tokio::test]
    async fn inline_parent_can_compose_and_expose_child_and_parent_handles() {
        let mut ctx = test_context();
        let parent = ctx
            .deploy_inline(InlineParentApp)
            .await
            .expect("deploy inline parent app");

        assert_eq!(parent.child, ChildHandle { id: "inline-child" });
        assert!(ctx.get::<ChildHandle>().is_some());
        assert!(ctx.get::<InlineParentHandle>().is_some());
    }

    fn test_context() -> DeployContext<TestEnv> {
        DeployContext::new(TestDeployment, NodeClients::default())
    }
}
