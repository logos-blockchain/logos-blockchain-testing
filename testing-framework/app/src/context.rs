use testing_framework_core::scenario::{
    Application, DynError, NodeClients, internal::CleanupGuard,
};
use testing_framework_runner_local::LocalDeployerEnv;

use crate::{
    AppDeployError, AppDeployment, AppHandle, HandleRegistry, LocalAppCluster,
    cleanup::AppCleanupStack,
};

/// Mutable deployment context used to compose applications and expose handles.
///
/// A context belongs to one scenario preparation. It provides access to the
/// outer scenario deployment and node clients while collecting application
/// handles for later use by workloads. Exposed handles are dropped in reverse
/// exposure order when the scenario runtime is released.
pub struct DeployContext<E: Application> {
    deployment: E::Deployment,
    node_clients: NodeClients<E>,
    handles: HandleRegistry,
    cleanup: AppCleanupStack,
}

impl<E> DeployContext<E>
where
    E: Application,
{
    /// Creates a context for an outer scenario deployment and its node clients.
    pub fn new(deployment: E::Deployment, node_clients: NodeClients<E>) -> Self {
        Self {
            deployment,
            node_clients,
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
        A: AppDeployment<E>,
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
        A: AppDeployment<E>,
    {
        let handle = self.deploy(app).await?;

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
    {
        let cluster = LocalAppCluster::start(deployment).await?;
        self.register_cleanup(cluster.cleanup_guard());
        Ok(cluster)
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
    use async_trait::async_trait;
    use testing_framework_core::{
        scenario::{Application, DynError, NodeClients},
        topology::DeploymentDescriptor,
    };

    use super::DeployContext;
    use crate::AppDeployment;

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

    fn test_context() -> DeployContext<TestEnv> {
        DeployContext::new(TestDeployment, NodeClients::default())
    }
}
