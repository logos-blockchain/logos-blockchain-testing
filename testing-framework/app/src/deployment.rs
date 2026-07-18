use async_trait::async_trait;
use testing_framework_core::scenario::{Application, DynError};
use testing_framework_runner_local::LocalClusterProvisioner;

use crate::DeployContext;

/// Typed runtime capability returned by an application deployment.
///
/// Handles are cloned when retrieved from the runtime registry. Managed
/// resources must be acquired through TF adapters, which register scenario
/// cleanup independently from these clones.
pub trait AppHandle: Clone + Send + Sync + 'static {}

impl<T> AppHandle for T where T: Clone + Send + Sync + 'static {}

/// Deploys one reusable application preset and returns its typed handle.
#[async_trait]
pub trait AppDeployment<E, P = LocalClusterProvisioner>: Send + 'static
where
    E: Application,
{
    /// Runtime capability produced by this deployment.
    type Handle: AppHandle;

    /// Prepares the application and returns its typed runtime access handle.
    ///
    /// Child deployments can be composed with [`DeployContext::deploy`] or
    /// [`DeployContext::deploy_and_expose`]. Any handle exposed through the
    /// context remains available through the scenario runtime.
    async fn deploy(self, ctx: &mut DeployContext<E, P>) -> Result<Self::Handle, DynError>;
}
