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

/// Deploys one application directly on a caller-owned local async path.
///
/// Unlike [`AppDeployment`], the future returned by `deploy_inline` is not
/// required to be `Send`. The deployment value and its handle retain the same
/// ownership requirements as the regular deployment contract, so the
/// difference is limited to the execution path of the deployment future.
#[async_trait(?Send)]
pub trait InlineAppDeployment<E, P = LocalClusterProvisioner>: Send + 'static
where
    E: Application,
{
    /// Runtime capability produced by this deployment.
    type Handle: AppHandle;

    /// Prepares the application without crossing a thread or task boundary.
    async fn deploy_inline(self, ctx: &mut DeployContext<E, P>) -> Result<Self::Handle, DynError>;
}
