use std::sync::Arc;

use async_trait::async_trait;
use kube::Client;
use testing_framework_core::scenario::{
    ClusterControlProfile, ClusterControlRequest, ClusterProvisioner, ClusterRequest,
    ClusterSource, ClusterStartMode, ClusterUnit, ClusterWaitHandle, DynError, ExistingCluster,
    ExternalNodeSource, MetricsError, NodeClients, NodeControlHandle, ObservabilityInputs,
};
use thiserror::Error;

use crate::{
    attach_provider::{K8sAttachProvider, K8sAttachedClusterWait},
    attached_control::K8sAttachedNodeControl,
    env::K8sDeployEnv,
    manual::{ManualCluster, ManualClusterError},
};

/// Kubernetes implementation of the shared cluster provisioning operation.
#[derive(Clone, Copy, Debug, Default)]
pub struct K8sClusterProvisioner;

#[derive(Debug, Error)]
/// Failures while provisioning a k8s cluster unit.
pub enum K8sClusterProvisionerError {
    #[error(transparent)]
    Manual(#[from] ManualClusterError),
    #[error(transparent)]
    Observability(#[from] MetricsError),
    #[error("failed to discover attached k8s cluster: {source}")]
    Attach {
        #[source]
        source: DynError,
    },
    #[error("failed to build external node client for '{name}': {source}")]
    ExternalNodeClient {
        name: String,
        #[source]
        source: DynError,
    },
    #[error("source orchestration failed: {source}")]
    Source {
        #[source]
        source: DynError,
    },
}

impl K8sClusterProvisionerError {
    /// Boxes the error while keeping cluster-availability failures
    /// downcastable to `ManualClusterError` (`ClientInit`/`InstallStack`).
    fn into_dyn(self) -> DynError {
        match self {
            Self::Manual(source) => source.into(),
            other => other.into(),
        }
    }
}

impl K8sClusterProvisioner {
    pub async fn provision<E: K8sDeployEnv>(
        &self,
        request: ClusterRequest<E>,
    ) -> Result<ClusterUnit<E>, K8sClusterProvisionerError> {
        match request.source().clone() {
            ClusterSource::Managed {
                deployment,
                external,
            } => provision_managed(&request, deployment, external).await,
            ClusterSource::Attached { cluster, external } => {
                provision_attached::<E>(&request, &cluster, external).await
            }
            ClusterSource::External { nodes } => {
                let clients = NodeClients::default();
                add_external_clients::<E>(&clients, nodes)?;
                Ok(ClusterUnit::new(
                    None,
                    clients,
                    ClusterControlProfile::ExternalUncontrolled,
                ))
            }
        }
    }
}

#[async_trait]
impl<E: K8sDeployEnv> ClusterProvisioner<E> for K8sClusterProvisioner {
    async fn provision_cluster(
        &self,
        request: ClusterRequest<E>,
    ) -> Result<ClusterUnit<E>, DynError> {
        self.provision(request)
            .await
            .map_err(K8sClusterProvisionerError::into_dyn)
    }
}

async fn provision_managed<E: K8sDeployEnv>(
    request: &ClusterRequest<E>,
    deployment: E::Deployment,
    external: Vec<ExternalNodeSource>,
) -> Result<ClusterUnit<E>, K8sClusterProvisionerError> {
    let observability =
        ObservabilityInputs::from_env()?.with_overrides(request.observability().clone());
    let cluster = Arc::new(
        ManualCluster::<E>::provision_named(
            deployment.clone(),
            request.name(),
            request.start_mode(),
            request.policy(),
            &observability,
        )
        .await?,
    );

    cluster
        .add_external_sources(external)
        .map_err(|source| K8sClusterProvisionerError::Source { source })?;

    let profile = match request.start_mode() {
        ClusterStartMode::Eager => ClusterControlProfile::FrameworkManaged,
        ClusterStartMode::OnDemand => ClusterControlProfile::ManualControlled,
    };
    let mut unit = ClusterUnit::new(Some(deployment), cluster.node_clients(), profile)
        .with_cluster_wait(Arc::clone(&cluster) as Arc<dyn ClusterWaitHandle>)
        .with_cleanup(cluster.cleanup_guard())
        .with_observability(observability)
        .with_attachment(cluster.attachment());

    if request.control() == ClusterControlRequest::Full {
        unit = unit.with_node_control(Arc::clone(&cluster) as Arc<dyn NodeControlHandle<E>>);
    }

    Ok(unit)
}

async fn provision_attached<E: K8sDeployEnv>(
    request: &ClusterRequest<E>,
    cluster: &ExistingCluster,
    external: Vec<ExternalNodeSource>,
) -> Result<ClusterUnit<E>, K8sClusterProvisionerError> {
    let client = init_kube_client().await?;
    let provider = K8sAttachProvider::<E>::new(client.clone());
    let attached = provider
        .discover(cluster)
        .await
        .map_err(|source| K8sClusterProvisionerError::Attach { source })?;

    let clients = NodeClients::default();
    for (_, node_client) in &attached.clients {
        clients.add_node(node_client.clone());
    }
    add_external_clients::<E>(&clients, external)?;

    let cluster_wait =
        K8sAttachedClusterWait::<E>::try_new(client.clone(), cluster, attached.access.clone())
            .map_err(|source| K8sClusterProvisionerError::Attach { source })?;

    let mut unit = ClusterUnit::new(
        None,
        clients,
        ClusterControlProfile::ExistingClusterAttached,
    )
    .with_cluster_wait(Arc::new(cluster_wait))
    .with_attachment(cluster.clone());

    if request.control() == ClusterControlRequest::Full {
        let control = K8sAttachedNodeControl::<E>::new(
            client,
            attached.namespace,
            attached.clients,
            attached.access,
        );
        unit = unit.with_node_control(Arc::new(control) as Arc<dyn NodeControlHandle<E>>);
    }

    if let Some(forwards) = attached.forwards {
        unit = unit.with_cleanup(forwards);
    }

    Ok(unit)
}

fn add_external_clients<E: K8sDeployEnv>(
    clients: &NodeClients<E>,
    sources: Vec<ExternalNodeSource>,
) -> Result<(), K8sClusterProvisionerError> {
    for source in sources {
        let client = E::external_node_client(&source).map_err(|error| {
            K8sClusterProvisionerError::ExternalNodeClient {
                name: source.label().to_owned(),
                source: error,
            }
        })?;
        clients.add_node(client);
    }
    Ok(())
}

async fn init_kube_client() -> Result<Client, K8sClusterProvisionerError> {
    crate::ensure_rustls_provider_installed();
    Client::try_default()
        .await
        .map_err(|source| ManualClusterError::ClientInit { source }.into())
}
