use async_trait::async_trait;
use testing_framework_core::scenario::{
    Application, CoreBuilderExt, DynError, NodeClients, PreparedRuntimeExtension, RunContext,
    RuntimeExtensionFactory, internal::CoreBuilderAccess,
};

use crate::{AppDeployment, AppHandle, AppRuntime, DeployContext};

/// Adapts an [`AppDeployment`] to the scenario runtime extension lifecycle.
///
/// A factory is normally installed through [`AppScenarioBuilderExt::with_app`]
/// rather than constructed directly.
pub struct AppDeploymentFactory<A> {
    app: A,
}

impl<A> AppDeploymentFactory<A> {
    /// Creates a runtime extension factory for `app`.
    pub const fn new(app: A) -> Self {
        Self { app }
    }
}

#[async_trait]
impl<E, A> RuntimeExtensionFactory<E> for AppDeploymentFactory<A>
where
    E: Application,
    A: AppDeployment<E> + Clone + Sync,
{
    async fn prepare(
        &self,
        deployment: &E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<PreparedRuntimeExtension, DynError> {
        let mut ctx = DeployContext::new(deployment.clone(), node_clients);

        let handle = ctx.deploy(self.app.clone()).await?;

        if !ctx.contains::<A::Handle>() {
            ctx.expose(handle)?;
        }

        let (handles, cleanup) = ctx.into_runtime_parts();
        let runtime = AppRuntime::new(handles);

        Ok(match cleanup {
            Some(cleanup) => PreparedRuntimeExtension::with_cleanup(runtime, cleanup),
            None => PreparedRuntimeExtension::new(runtime),
        })
    }
}

/// Adds composable application deployments to scenario builders.
pub trait AppScenarioBuilderExt: CoreBuilderAccess + Sized {
    /// Registers an application deployment to prepare before workloads start.
    ///
    /// If the root deployment does not expose its returned handle itself, the
    /// factory exposes it as the default handle automatically.
    #[must_use]
    fn with_app<A>(self, app: A) -> Self
    where
        A: AppDeployment<Self::Env> + Clone + Sync,
    {
        self.with_runtime_extension_factory(Box::new(AppDeploymentFactory::new(app)))
    }
}

impl<T> AppScenarioBuilderExt for T where T: CoreBuilderAccess {}

/// Retrieves application handles from a running scenario.
pub trait AppRunContextExt<E: Application> {
    /// Returns the default handle for `T`, or `None` if it is not exposed.
    fn app<T>(&self) -> Option<T>
    where
        T: AppHandle;

    /// Returns a named handle for `T`, or `None` if it is not exposed.
    fn app_named<T>(&self, name: &str) -> Option<T>
    where
        T: AppHandle;

    /// Returns the default handle for `T` or an error when it is unavailable.
    fn require_app<T>(&self) -> Result<T, DynError>
    where
        T: AppHandle;

    /// Returns a named handle for `T` or an error when it is unavailable.
    fn require_app_named<T>(&self, name: &str) -> Result<T, DynError>
    where
        T: AppHandle;
}

impl<E> AppRunContextExt<E> for RunContext<E>
where
    E: Application,
{
    fn app<T>(&self) -> Option<T>
    where
        T: AppHandle,
    {
        self.extension::<AppRuntime>()?.get()
    }

    fn app_named<T>(&self, name: &str) -> Option<T>
    where
        T: AppHandle,
    {
        self.extension::<AppRuntime>()?.get_named(name)
    }

    fn require_app<T>(&self) -> Result<T, DynError>
    where
        T: AppHandle,
    {
        self.require_extension::<AppRuntime>()?
            .require()
            .map_err(Into::into)
    }

    fn require_app_named<T>(&self, name: &str) -> Result<T, DynError>
    where
        T: AppHandle,
    {
        self.require_extension::<AppRuntime>()?
            .require_named(name)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use async_trait::async_trait;
    use testing_framework_core::{
        scenario::{Application, DynError, NodeClients, RuntimeExtensionFactory},
        topology::DeploymentDescriptor,
    };

    use super::AppDeploymentFactory;
    use crate::{AppDeployment, DeployContext};

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

    #[derive(Clone)]
    struct FailingApp {
        dropped: Arc<AtomicBool>,
    }

    #[derive(Clone)]
    struct OwnedHandle {
        _resource: Arc<DropProbe>,
    }

    struct DropProbe {
        dropped: Arc<AtomicBool>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl AppDeployment<TestEnv> for FailingApp {
        type Handle = OwnedHandle;

        async fn deploy(self, ctx: &mut DeployContext<TestEnv>) -> Result<Self::Handle, DynError> {
            ctx.expose(OwnedHandle {
                _resource: Arc::new(DropProbe {
                    dropped: Arc::clone(&self.dropped),
                }),
            })?;
            Err("deployment failed".into())
        }
    }

    #[tokio::test]
    async fn deployment_failure_cleans_up_partial_resources() {
        let dropped = Arc::new(AtomicBool::new(false));
        let factory = AppDeploymentFactory::new(FailingApp {
            dropped: Arc::clone(&dropped),
        });

        let result = factory
            .prepare(test_deployment(), NodeClients::default())
            .await;

        assert!(result.is_err(), "deployment must fail");

        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn app_factory_can_prepare_more_than_once() {
        let dropped = Arc::new(AtomicBool::new(false));
        let factory = AppDeploymentFactory::new(FailingApp {
            dropped: Arc::clone(&dropped),
        });

        for _ in 0..2 {
            let result = factory
                .prepare(test_deployment(), NodeClients::default())
                .await;
            assert!(result.is_err(), "deployment must fail");
        }
    }

    fn test_deployment() -> &'static TestDeployment {
        static DEPLOYMENT: TestDeployment = TestDeployment;
        &DEPLOYMENT
    }
}
