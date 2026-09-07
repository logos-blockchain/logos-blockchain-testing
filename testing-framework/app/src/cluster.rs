use async_trait::async_trait;
use testing_framework_core::scenario::{
    Application, ClusterControlRequest, ClusterHandle, ClusterProvisioner, ClusterRequest,
    ClusterStartMode, DeploymentPolicy, DynError, ExternalNodeSource, ObservabilityInputs,
};

use crate::{AppHostEnv, DeployContext, deployment::AppDeployment};

/// Uniform node cluster deployed as an ordinary application.
///
/// The backend is chosen by the cluster provisioner the scenario supplies, so
/// the same deployment runs on local processes, Docker Compose, or Kubernetes
/// via `with_app_using`. The exposed handle is the backend-neutral
/// [`ClusterHandle`].
pub struct ClusterApp<E: Application> {
    request: ClusterRequest<E>,
}

impl<E: Application> Clone for ClusterApp<E> {
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
        }
    }
}

impl<E: Application> ClusterApp<E> {
    /// Creates a managed cluster application for the given deployment.
    #[must_use]
    pub fn new(deployment: E::Deployment) -> Self {
        Self {
            request: ClusterRequest::managed(deployment).with_control(ClusterControlRequest::Full),
        }
    }

    /// Assigns a stable cluster name so several managed clusters can share one
    /// deployment session; backends namespace the cluster's services with it.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.request = self.request.with_name(name);
        self
    }

    /// Overrides the deployment policy applied to the cluster request.
    #[must_use]
    pub fn with_policy(mut self, policy: DeploymentPolicy) -> Self {
        self.request = self.request.with_policy(policy);
        self
    }

    /// Overrides the start mode requested from the backend provisioner.
    #[must_use]
    pub fn with_start_mode(mut self, start_mode: ClusterStartMode) -> Self {
        self.request = self.request.with_start_mode(start_mode);
        self
    }

    /// Overrides the runtime control requested for the cluster.
    #[must_use]
    pub fn with_control(mut self, control: ClusterControlRequest) -> Self {
        self.request = self.request.with_control(control);
        self
    }

    /// Adds external nodes joined to the managed cluster inventory.
    #[must_use]
    pub fn with_external_nodes(mut self, nodes: Vec<ExternalNodeSource>) -> Self {
        self.request = self.request.with_external_nodes(nodes);
        self
    }

    /// Overrides the observability endpoints supplied to the backend.
    #[must_use]
    pub fn with_observability(mut self, observability: ObservabilityInputs) -> Self {
        self.request = self.request.with_observability(observability);
        self
    }

    fn into_request(self) -> ClusterRequest<E> {
        self.request
    }
}

#[async_trait]
impl<E, P> AppDeployment<AppHostEnv, P> for ClusterApp<E>
where
    E: Application,
    P: ClusterProvisioner<E>,
{
    type Handle = ClusterHandle<E>;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, P>,
    ) -> Result<Self::Handle, DynError> {
        ctx.deploy_cluster(self.into_request()).await
    }
}

#[cfg(test)]
mod tests {
    use testing_framework_core::{
        scenario::{
            ClusterControlRequest, ClusterSource, ClusterStartMode, DeploymentPolicy,
            ExternalNodeSource,
        },
        topology::NodeCountTopology,
    };

    use super::ClusterApp;

    struct TestEnv;

    #[async_trait::async_trait]
    impl testing_framework_core::scenario::Application for TestEnv {
        type Deployment = NodeCountTopology;
        type NodeClient = u8;
        type NodeConfig = ();
    }

    #[test]
    fn cluster_app_builds_a_managed_full_control_request_by_default() {
        let request = ClusterApp::<TestEnv>::new(NodeCountTopology::new(3)).into_request();

        assert!(matches!(request.source(), ClusterSource::Managed { .. }));
        assert_eq!(request.name(), None);
        assert_eq!(request.control(), ClusterControlRequest::Full);
        assert_eq!(request.start_mode(), ClusterStartMode::Eager);
        assert_eq!(request.policy(), DeploymentPolicy::default());
    }

    #[test]
    fn cluster_app_forwards_overrides_into_the_request() {
        let policy = DeploymentPolicy {
            readiness_enabled: false,
            ..DeploymentPolicy::default()
        };
        let request = ClusterApp::<TestEnv>::new(NodeCountTopology::new(1))
            .with_name("alpha")
            .with_policy(policy)
            .with_start_mode(ClusterStartMode::OnDemand)
            .with_control(ClusterControlRequest::None)
            .with_external_nodes(vec![ExternalNodeSource::new(
                "external-0".to_owned(),
                "http://127.0.0.1:1".to_owned(),
            )])
            .into_request();

        assert_eq!(request.name(), Some("alpha"));
        assert_eq!(request.policy(), policy);
        assert_eq!(request.start_mode(), ClusterStartMode::OnDemand);
        assert_eq!(request.control(), ClusterControlRequest::None);
        let ClusterSource::Managed { external, .. } = request.source() else {
            panic!("managed request changed source kind");
        };
        assert_eq!(external.len(), 1);
    }
}
