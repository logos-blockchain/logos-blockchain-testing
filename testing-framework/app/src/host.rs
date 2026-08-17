use async_trait::async_trait;
use testing_framework_core::{
    scenario::{Application, DynError, NodeAccess, NodeClients, ScenarioBuilder},
    topology::DeploymentDescriptor,
};
use testing_framework_runner_local::{LocalDeployerEnv, ProcessDeployer};

use crate::{InlineAppDeployment, InlineAppDeploymentFactory, InlineAppRuntime};

#[derive(Clone, Default)]
/// Empty outer topology for scenarios whose entire system is deployed as apps.
///
/// Application deployments registered with [`crate::AppScenarioBuilderExt`]
/// provide all processes and clusters, so this topology contains no nodes.
pub struct AppHostTopology;

impl DeploymentDescriptor for AppHostTopology {
    fn node_count(&self) -> usize {
        0
    }
}

/// Testing-framework application environment for [`AppHost`] scenarios.
///
/// This environment intentionally has no outer node client.
/// Application-specific clients are exposed as typed handles instead.
pub struct AppHostEnv;

#[async_trait]
impl Application for AppHostEnv {
    type Deployment = AppHostTopology;
    type NodeClient = ();
    type NodeConfig = ();

    fn build_node_client(_access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Err(std::io::Error::other("app host does not expose node clients").into())
    }
}

#[async_trait]
impl LocalDeployerEnv for AppHostEnv {}

/// Entry point for a scenario composed entirely from application deployments.
pub struct AppHost;

impl AppHost {
    /// Creates an empty scenario builder ready for `.with_app(...)` calls.
    #[must_use]
    pub fn scenario() -> AppHostScenarioBuilder {
        ScenarioBuilder::with_deployment(AppHostTopology)
    }

    /// Deploys a root application directly on the caller's local async path.
    ///
    /// This is the explicit root entrypoint for deployments whose future is
    /// not `Send`. It does not register a `RuntimeExtensionFactory`, so it can
    /// be used by local in-process harnesses that own the deployment lifetime
    /// themselves. The returned runtime owns application cleanup until it is
    /// dropped.
    pub async fn deploy_inline<A>(app: A) -> Result<InlineAppRuntime, DynError>
    where
        A: InlineAppDeployment<AppHostEnv>,
    {
        Self::deploy_inline_using(app, testing_framework_runner_local::LocalClusterProvisioner)
            .await
    }

    /// Deploys a root application inline with an explicit local provisioner.
    pub async fn deploy_inline_using<A, P>(
        app: A,
        provisioner: P,
    ) -> Result<InlineAppRuntime, DynError>
    where
        A: InlineAppDeployment<AppHostEnv, P>,
        P: Send + Sync + 'static,
    {
        InlineAppDeploymentFactory::with_provisioner(app, provisioner)
            .prepare_inline(AppHostTopology, NodeClients::default())
            .await
    }
}

/// Scenario builder for an application-hosted heterogeneous stack.
pub type AppHostScenarioBuilder = ScenarioBuilder<AppHostEnv>;
/// Local process deployer used to execute an [`AppHost`] scenario.
pub type AppHostLocalDeployer = ProcessDeployer<AppHostEnv>;

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use async_trait::async_trait;
    use testing_framework_core::scenario::DynError;

    use super::{AppHost, AppHostEnv};
    use crate::{DeployContext, InlineAppDeployment};

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct RootHandle;

    struct RootApp;

    #[async_trait(?Send)]
    impl InlineAppDeployment<AppHostEnv> for RootApp {
        type Handle = RootHandle;

        async fn deploy_inline(
            self,
            _ctx: &mut DeployContext<AppHostEnv>,
        ) -> Result<Self::Handle, DynError> {
            let marker = Rc::new("root");
            tokio::task::yield_now().await;
            assert_eq!(*marker, "root");

            Ok(RootHandle)
        }
    }

    #[tokio::test]
    async fn app_host_has_an_inline_root_entrypoint() {
        let runtime = AppHost::deploy_inline(RootApp)
            .await
            .expect("deploy inline root app");

        assert_eq!(runtime.app::<RootHandle>(), Some(RootHandle));
    }
}
