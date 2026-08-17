use testing_framework_core::scenario::{
    Application, DynError, NodeClients, internal::CleanupGuard,
};
use testing_framework_runner_local::LocalClusterProvisioner;

use crate::{AppDeployError, AppHandle, AppRuntime, DeployContext, InlineAppDeployment};

/// Owns one inline application deployment and its managed cleanup.
///
/// The runtime contains the same exposed handles as the regular
/// [`AppRuntime`] scenario extension. Dropping this value runs cleanup for
/// resources acquired during deployment.
pub struct InlineAppRuntime {
    runtime: AppRuntime,
    cleanup: Option<Box<dyn CleanupGuard>>,
}

impl InlineAppRuntime {
    pub(crate) fn new(runtime: AppRuntime, cleanup: Option<Box<dyn CleanupGuard>>) -> Self {
        Self { runtime, cleanup }
    }

    /// Borrows the prepared application runtime and its exposed handles.
    #[must_use]
    pub fn runtime(&self) -> &AppRuntime {
        &self.runtime
    }

    /// Returns the default handle for `T`, if it was exposed.
    #[must_use]
    pub fn app<T>(&self) -> Option<T>
    where
        T: AppHandle,
    {
        self.runtime.get()
    }

    /// Returns a named handle for `T`, if it was exposed.
    #[must_use]
    pub fn app_named<T>(&self, name: &str) -> Option<T>
    where
        T: AppHandle,
    {
        self.runtime.get_named(name)
    }

    /// Returns the default handle for `T` or a typed missing-handle error.
    pub fn require_app<T>(&self) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.runtime.require()
    }

    /// Returns a named handle for `T` or a typed missing-handle error.
    pub fn require_app_named<T>(&self, name: &str) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.runtime.require_named(name)
    }
}

impl Drop for InlineAppRuntime {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup.cleanup();
        }
    }
}

/// One-shot adapter for preparing an [`InlineAppDeployment`] outside the
/// `RuntimeExtensionFactory` API.
///
/// This type deliberately does not implement `RuntimeExtensionFactory`:
/// inline deployment is a caller-owned local path whose future may be
/// non-`Send`. Consuming the factory also means an inline app is prepared once,
/// rather than cloned for repeated runtime-extension preparation.
pub struct InlineAppDeploymentFactory<A, P = LocalClusterProvisioner> {
    app: A,
    provisioner: P,
}

impl<A> InlineAppDeploymentFactory<A, LocalClusterProvisioner> {
    /// Creates an inline deployment helper using the default local
    /// provisioner.
    pub const fn new(app: A) -> Self {
        Self {
            app,
            provisioner: LocalClusterProvisioner,
        }
    }
}

impl<A, P> InlineAppDeploymentFactory<A, P> {
    /// Creates an inline deployment helper using an explicit provisioner.
    pub const fn with_provisioner(app: A, provisioner: P) -> Self {
        Self { app, provisioner }
    }

    /// Prepares the application on the caller's async path.
    pub async fn prepare_inline<E>(
        self,
        deployment: E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<InlineAppRuntime, DynError>
    where
        E: Application,
        A: InlineAppDeployment<E, P>,
        P: Send + Sync + 'static,
    {
        let mut ctx =
            DeployContext::new_with_provisioner(deployment, node_clients, self.provisioner);

        let handle = ctx.deploy_inline(self.app).await?;

        if !ctx.contains::<A::Handle>() {
            ctx.expose(handle)?;
        }

        let (handles, cleanup) = ctx.into_runtime_parts();
        Ok(InlineAppRuntime::new(AppRuntime::new(handles), cleanup))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use testing_framework_core::scenario::internal::CleanupGuard;

    use super::InlineAppRuntime;
    use crate::{AppRuntime, HandleRegistry};

    struct CleanupProbe(Arc<AtomicBool>);

    impl CleanupGuard for CleanupProbe {
        fn cleanup(self: Box<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn inline_runtime_runs_cleanup_when_dropped() {
        let cleaned = Arc::new(AtomicBool::new(false));
        let runtime = InlineAppRuntime::new(
            AppRuntime::new(HandleRegistry::new()),
            Some(Box::new(CleanupProbe(Arc::clone(&cleaned)))),
        );

        drop(runtime);

        assert!(cleaned.load(Ordering::SeqCst));
    }
}
