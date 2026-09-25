use std::sync::{Mutex, PoisonError};

use testing_framework_core::scenario::{CleanupGuard, DynError, NodeClients};
use testing_framework_runner_local::LocalClusterProvisioner;

use crate::{AppDeployment, AppHandle, AppHostEnv, AppHostTopology, AppRuntime, DeployContext};

/// Deploys an application without creating a scenario or running workloads.
#[derive(Clone, Default)]
pub struct AppDeployer<P = LocalClusterProvisioner> {
    provisioner: P,
}

impl AppDeployer {
    /// Uses the local cluster provisioner.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            provisioner: LocalClusterProvisioner,
        }
    }
}

impl<P> AppDeployer<P> {
    /// Uses the supplied backend for resources requested by the application.
    #[must_use]
    pub const fn with_provisioner(provisioner: P) -> Self {
        Self { provisioner }
    }
}

impl<P: Clone + Send + Sync + 'static> AppDeployer<P> {
    /// Deploys the app and retains its handles and cleanup until the result
    /// drops.
    pub async fn deploy<A>(&self, app: A) -> Result<DeployedApp<A::Handle>, DynError>
    where
        A: AppDeployment<AppHostEnv, P>,
    {
        let mut ctx = DeployContext::new_with_provisioner(
            AppHostTopology,
            NodeClients::default(),
            self.provisioner.clone(),
        );
        let handle = ctx.deploy(app).await?;
        if !ctx.contains::<A::Handle>() {
            ctx.expose(handle.clone())?;
        }
        let (handles, cleanup, _, _) = ctx.into_runtime_parts();

        Ok(DeployedApp {
            handle,
            runtime: AppRuntime::new(handles),
            cleanup: Mutex::new(cleanup),
        })
    }
}

/// Owns a deployed application's handles and managed resources.
///
/// Dropping this owner runs cleanup even if callers retain cloned handles.
/// Keep it alive while using the application. Failed or cancelled deployments
/// clean up resources acquired before returning this owner.
pub struct DeployedApp<H: AppHandle> {
    handle: H,
    runtime: AppRuntime,
    // CleanupGuard is Send, but not Sync. Keep the owner shareable without
    // imposing a stronger bound on existing application cleanup guards.
    cleanup: Mutex<Option<Box<dyn CleanupGuard>>>,
}

impl<H: AppHandle> DeployedApp<H> {
    /// Returns the application's primary handle.
    #[must_use]
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Returns the handles exposed by the app and its child deployments.
    #[must_use]
    pub const fn runtime(&self) -> &AppRuntime {
        &self.runtime
    }
}

impl<H: AppHandle> Drop for DeployedApp<H> {
    fn drop(&mut self) {
        if let Some(cleanup) = self
            .cleanup
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            cleanup.cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{future::pending, sync::Arc, time::Duration};

    use async_trait::async_trait;
    use testing_framework_core::scenario::{ClusterProvisioner, ClusterRequest, ClusterUnit};
    use tokio::time::timeout;

    use super::*;
    use crate::ClusterApp;

    struct RecordCleanup {
        events: Arc<Mutex<Vec<&'static str>>>,
        name: &'static str,
    }

    impl CleanupGuard for RecordCleanup {
        fn cleanup(self: Box<Self>) {
            self.events.lock().unwrap().push(self.name);
        }
    }

    #[derive(Clone)]
    struct Handle(Arc<Mutex<Vec<&'static str>>>);

    enum Outcome {
        Success,
        Failure,
        Pending,
    }

    struct TestApp {
        events: Arc<Mutex<Vec<&'static str>>>,
        outcome: Outcome,
    }

    #[async_trait]
    impl AppDeployment<AppHostEnv> for TestApp {
        type Handle = Handle;

        async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Handle, DynError> {
            for name in ["dependency", "app"] {
                ctx.defer_cleanup(Box::new(RecordCleanup {
                    events: Arc::clone(&self.events),
                    name,
                }));
            }
            ctx.expose_named("child", 42_u32)?;
            match self.outcome {
                Outcome::Success => Ok(Handle(self.events)),
                Outcome::Failure => Err("deployment failed".into()),
                Outcome::Pending => pending().await,
            }
        }
    }

    #[tokio::test]
    async fn owner_keeps_resources_until_drop_even_with_cloned_handles() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DeployedApp<Handle>>();

        let events = Arc::new(Mutex::new(Vec::new()));
        let deployed = AppDeployer::new()
            .deploy(TestApp {
                events: Arc::clone(&events),
                outcome: Outcome::Success,
            })
            .await
            .unwrap();
        let handle = deployed.handle().clone();
        let exposed = deployed.runtime().require::<Handle>().unwrap();
        assert_eq!(deployed.runtime().get_named::<u32>("child"), Some(42));
        assert!(events.lock().unwrap().is_empty());

        drop(deployed);
        assert_eq!(*handle.0.lock().unwrap(), ["app", "dependency"]);
        drop(exposed);
        drop(handle);
        assert_eq!(*events.lock().unwrap(), ["app", "dependency"]);
    }

    #[tokio::test]
    async fn failed_deployment_cleans_up_partial_resources() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let result = AppDeployer::new()
            .deploy(TestApp {
                events: Arc::clone(&events),
                outcome: Outcome::Failure,
            })
            .await;
        assert_eq!(result.err().unwrap().to_string(), "deployment failed");
        assert_eq!(*events.lock().unwrap(), ["app", "dependency"]);
    }

    #[tokio::test]
    async fn cancelled_deployment_cleans_up_partial_resources() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let deployer = AppDeployer::new();
        let deploying = deployer.deploy(TestApp {
            events: Arc::clone(&events),
            outcome: Outcome::Pending,
        });
        assert!(timeout(Duration::from_millis(10), deploying).await.is_err());
        assert_eq!(*events.lock().unwrap(), ["app", "dependency"]);
    }

    #[derive(Clone)]
    struct CustomProvisioner(u32);

    #[async_trait]
    impl ClusterProvisioner<AppHostEnv> for CustomProvisioner {
        async fn provision_cluster(
            &self,
            _request: ClusterRequest<AppHostEnv>,
        ) -> Result<ClusterUnit<AppHostEnv>, DynError> {
            Err(format!("backend {} failed", self.0).into())
        }
    }

    #[tokio::test]
    async fn uses_the_selected_backend() {
        let result = AppDeployer::with_provisioner(CustomProvisioner(7))
            .deploy(ClusterApp::<AppHostEnv>::new(AppHostTopology))
            .await;
        assert_eq!(result.err().unwrap().to_string(), "backend 7 failed");
    }
}
