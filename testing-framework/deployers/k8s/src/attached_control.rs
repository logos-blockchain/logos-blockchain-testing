use k8s_openapi::api::{apps::v1::Deployment, core::v1::Service};
use kube::{Api, Client, Error as KubeError};
use testing_framework_core::scenario::{
    DEFAULT_READINESS_POLL_INTERVAL, DEFAULT_READINESS_TIMEOUT, DynError, NodeAccess, NodeControl,
    NodeControlHandle, NodeLaunchOptions, ReadinessRequirement, StartNodeOptions, StartedNode,
    StartedNodeAccess, wait_for_readiness_ports,
};
use thiserror::Error;

use crate::{
    attach_provider::{AttachedAccess, extract_api_node_port},
    env::K8sDeployEnv,
    host::node_host,
    manual::{
        ManualClusterError, ensure_default_cfgsync_options, scale_deployment,
        validate_restart_options,
    },
};

const LOCALHOST: &str = "127.0.0.1";

/// Failures of attached k8s node control operations.
///
/// The control handle is always granted when full control is requested on an
/// attached cluster; workload-shape mismatches surface here at call time
/// instead.
#[derive(Debug, Error)]
pub(crate) enum K8sAttachedControlError {
    #[error(
        "node '{name}' is not part of the attached cluster in namespace '{namespace}'; known \
         nodes: [{known}]"
    )]
    UnknownNode {
        name: String,
        namespace: String,
        known: String,
    },
    #[error("failed to resolve deployment '{deployment}' in namespace '{namespace}': {source}")]
    ResolveDeployment {
        deployment: String,
        namespace: String,
        #[source]
        source: KubeError,
    },
    #[error(
        "no deployment '{deployment}' exists in namespace '{namespace}'; attached node control \
         requires one deployment named after each node service"
    )]
    DeploymentNotFound {
        deployment: String,
        namespace: String,
    },
    #[error(
        "deployment '{deployment}' in namespace '{namespace}' declares {replicas} replicas; \
         attached node control requires at most 1"
    )]
    UnexpectedReplicas {
        deployment: String,
        namespace: String,
        replicas: i32,
    },
    #[error("failed to scale deployment '{deployment}' in namespace '{namespace}': {source}")]
    Scale {
        deployment: String,
        namespace: String,
        #[source]
        source: ManualClusterError,
    },
    #[error(
        "failed to re-establish the port-forward for service '{service}' in namespace \
         '{namespace}': {source}"
    )]
    RespawnForward {
        service: String,
        namespace: String,
        #[source]
        source: DynError,
    },
    #[error("service '{service}' in namespace '{namespace}' has no registered port-forward")]
    MissingForward { service: String, namespace: String },
    #[error("failed to resolve service '{service}' in namespace '{namespace}': {source}")]
    ResolveService {
        service: String,
        namespace: String,
        #[source]
        source: DynError,
    },
    #[error("node readiness failed for '{name}': {source}")]
    NodeReadiness {
        name: String,
        #[source]
        source: DynError,
    },
}

/// Node control for attached k8s clusters.
///
/// Reuses the managed path's restart-by-scaling machinery: a restart scales
/// the node's deployment to zero, waits for its pods to be gone, and scales
/// it back to one. Stop and start drive the same machinery on their own: stop
/// scales to zero, start scales back to one, re-establishes the node's
/// port-forward, and waits for readiness. The framework's convention names
/// each node's deployment after its discovered service, so the deployment is
/// resolved by that name in the attached namespace at call time.
pub(crate) struct K8sAttachedNodeControl<E: K8sDeployEnv> {
    client: Client,
    namespace: String,
    nodes: Vec<(String, E::NodeClient)>,
    access: AttachedAccess,
}

impl<E: K8sDeployEnv> K8sAttachedNodeControl<E> {
    pub(crate) const fn new(
        client: Client,
        namespace: String,
        nodes: Vec<(String, E::NodeClient)>,
        access: AttachedAccess,
    ) -> Self {
        Self {
            client,
            namespace,
            nodes,
            access,
        }
    }

    async fn restart(&self, name: &str) -> Result<(), K8sAttachedControlError> {
        self.resolve_deployment(name).await?;
        self.scale(name, 0).await?;
        self.scale(name, 1).await?;
        self.restore_forward(name).await
    }

    /// Stops the node by scaling its deployment to zero and waiting for its
    /// pods to be gone.
    async fn stop(&self, name: &str) -> Result<(), K8sAttachedControlError> {
        self.resolve_deployment(name).await?;
        self.scale(name, 0).await
    }

    /// Starts the node by scaling its deployment back to one, re-establishing
    /// its port-forward, and waiting for readiness.
    async fn start(&self, name: &str) -> Result<(), K8sAttachedControlError> {
        self.resolve_deployment(name).await?;
        self.scale(name, 1).await?;
        self.restore_forward(name).await?;
        self.wait_ready(name).await
    }

    fn started_node(&self, name: &str) -> Result<StartedNode<E>, K8sAttachedControlError> {
        self.nodes
            .iter()
            .find(|(node, _)| node == name)
            .map(|(node, client)| StartedNode {
                name: node.clone(),
                client: client.clone(),
            })
            .ok_or_else(|| self.unknown_node_error(name))
    }

    async fn access(&self, name: &str) -> Result<NodeAccess, K8sAttachedControlError> {
        self.require_known_node(name)?;
        let (host, port) = match &self.access {
            AttachedAccess::Direct => (node_host(), self.resolve_node_port(name).await?),
            AttachedAccess::Forwarded { forwards } => {
                let port = forwards.local_port(name).ok_or_else(|| {
                    K8sAttachedControlError::MissingForward {
                        service: name.to_owned(),
                        namespace: self.namespace.clone(),
                    }
                })?;
                (LOCALHOST.to_owned(), port)
            }
        };

        Ok(NodeAccess::new(host, port))
    }

    async fn wait_ready(&self, name: &str) -> Result<(), K8sAttachedControlError> {
        let access = self.access(name).await?;
        wait_for_readiness_ports(
            &[access.api_port()],
            access.host(),
            E::node_readiness_probe(),
            ReadinessRequirement::AllNodesReady,
            DEFAULT_READINESS_TIMEOUT,
            DEFAULT_READINESS_POLL_INTERVAL,
        )
        .await
        .map_err(|source| K8sAttachedControlError::NodeReadiness {
            name: name.to_owned(),
            source: source.into(),
        })
    }

    /// Resolves the node's deployment by its service name and rejects
    /// workload shapes the restart-by-scaling mechanism cannot drive.
    async fn resolve_deployment(&self, name: &str) -> Result<(), K8sAttachedControlError> {
        self.require_known_node(name)?;
        let deployments = Api::<Deployment>::namespaced(self.client.clone(), &self.namespace);
        match deployments.get(name).await {
            Ok(deployment) => validate_deployment_shape(&deployment, name, &self.namespace),
            Err(KubeError::Api(response)) if response.code == 404 => {
                Err(K8sAttachedControlError::DeploymentNotFound {
                    deployment: name.to_owned(),
                    namespace: self.namespace.clone(),
                })
            }
            Err(source) => Err(K8sAttachedControlError::ResolveDeployment {
                deployment: name.to_owned(),
                namespace: self.namespace.clone(),
                source,
            }),
        }
    }

    async fn resolve_node_port(&self, name: &str) -> Result<u16, K8sAttachedControlError> {
        let resolve_error = |source: DynError| K8sAttachedControlError::ResolveService {
            service: name.to_owned(),
            namespace: self.namespace.clone(),
            source,
        };
        let services = Api::<Service>::namespaced(self.client.clone(), &self.namespace);
        let service = services
            .get(name)
            .await
            .map_err(|source| resolve_error(source.into()))?;
        extract_api_node_port(&service).map_err(resolve_error)
    }

    async fn scale(&self, name: &str, replicas: i32) -> Result<(), K8sAttachedControlError> {
        scale_deployment(&self.client, &self.namespace, name, name, replicas)
            .await
            .map_err(|source| K8sAttachedControlError::Scale {
                deployment: name.to_owned(),
                namespace: self.namespace.clone(),
                source,
            })
    }

    /// Re-establishes the node's port-forward after its pod was replaced.
    ///
    /// A pod replacement kills the `kubectl port-forward` bound to it, so
    /// forwarded access respawns the service's forward on its original local
    /// port; direct `NodePort` access needs nothing.
    async fn restore_forward(&self, name: &str) -> Result<(), K8sAttachedControlError> {
        let AttachedAccess::Forwarded { forwards } = &self.access else {
            return Ok(());
        };

        let respawn_error = |source: DynError| K8sAttachedControlError::RespawnForward {
            service: name.to_owned(),
            namespace: self.namespace.clone(),
            source,
        };
        let forwards = forwards.clone();
        let service = name.to_owned();
        tokio::task::spawn_blocking(move || forwards.respawn(&service))
            .await
            .map_err(|source| respawn_error(format!("port-forward task failed: {source}").into()))?
            .map_err(respawn_error)
    }

    fn require_known_node(&self, name: &str) -> Result<(), K8sAttachedControlError> {
        if self.nodes.iter().any(|(node, _)| node == name) {
            return Ok(());
        }

        Err(self.unknown_node_error(name))
    }

    fn unknown_node_error(&self, name: &str) -> K8sAttachedControlError {
        K8sAttachedControlError::UnknownNode {
            name: name.to_owned(),
            namespace: self.namespace.clone(),
            known: self
                .nodes
                .iter()
                .map(|(node, _)| node.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

fn validate_deployment_shape(
    deployment: &Deployment,
    name: &str,
    namespace: &str,
) -> Result<(), K8sAttachedControlError> {
    let replicas = deployment
        .spec
        .as_ref()
        .and_then(|spec| spec.replicas)
        .unwrap_or(1);
    if replicas == 0 || replicas == 1 {
        return Ok(());
    }

    Err(K8sAttachedControlError::UnexpectedReplicas {
        deployment: name.to_owned(),
        namespace: namespace.to_owned(),
        replicas,
    })
}

#[async_trait::async_trait]
impl<E: K8sDeployEnv> NodeControl for K8sAttachedNodeControl<E> {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.restart(name).await.map_err(Into::into)
    }

    async fn restart_node_with(
        &self,
        name: &str,
        options: NodeLaunchOptions,
    ) -> Result<(), DynError> {
        self.restart_node_with_config(name, options.into()).await
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: NodeLaunchOptions,
    ) -> Result<StartedNodeAccess, DynError> {
        let started = self.start_node_with_config(name, options.into()).await?;
        let access = NodeControl::node_access(self, &started.name).await?;
        Ok(StartedNodeAccess {
            name: started.name,
            access,
        })
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.stop(name).await.map_err(Into::into)
    }

    async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        self.wait_ready(name).await.map_err(Into::into)
    }

    async fn node_access(&self, name: &str) -> Result<NodeAccess, DynError> {
        self.access(name).await.map_err(Into::into)
    }

    fn node_names(&self) -> Vec<String> {
        self.nodes.iter().map(|(node, _)| node.clone()).collect()
    }
}

#[async_trait::async_trait]
impl<E: K8sDeployEnv> NodeControlHandle<E> for K8sAttachedNodeControl<E> {
    async fn restart_node_with_config(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        validate_restart_options(&options)?;
        ensure_default_cfgsync_options(&options)?;
        self.restart(name).await.map_err(Into::into)
    }

    async fn start_node_with_config(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        validate_restart_options(&options)?;
        ensure_default_cfgsync_options(&options)?;
        self.start(name).await?;
        self.started_node(name).map_err(Into::into)
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.nodes
            .iter()
            .find(|(node, _)| node == name)
            .map(|(_, client)| client.clone())
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
    use kube::{Client, Config};
    use testing_framework_core::scenario::{
        DynError, NodeAccess, NodeControl, NodeControlHandle, NodeLaunchOptions, StartNodeOptions,
    };

    use super::{K8sAttachedControlError, K8sAttachedNodeControl, validate_deployment_shape};
    use crate::{
        attach_provider::{AttachedAccess, AttachedForwardRegistry},
        manual::tests_dummy_env::DummyEnv,
    };

    fn offline_control(access: AttachedAccess) -> K8sAttachedNodeControl<DummyEnv> {
        crate::ensure_rustls_provider_installed();
        let config = Config::new("http://127.0.0.1:1".parse().expect("cluster url"));
        let client = Client::try_from(config).expect("offline kube client");
        K8sAttachedNodeControl::new(
            client,
            "attached-ns".to_owned(),
            vec![("kv-node-0".to_owned(), "http://kv-node-0/".to_owned())],
            access,
        )
    }

    fn expect_start_error<T>(result: Result<T, DynError>, msg: &str) -> DynError {
        match result {
            Ok(_) => panic!("{msg}"),
            Err(error) => error,
        }
    }

    fn deployment_with_replicas(replicas: Option<i32>) -> Deployment {
        Deployment {
            spec: Some(DeploymentSpec {
                replicas,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn deployment_shape_accepts_single_replica() {
        let explicit = deployment_with_replicas(Some(1));
        let defaulted = deployment_with_replicas(None);

        assert!(validate_deployment_shape(&explicit, "kv-node-0", "attached-ns").is_ok());
        assert!(validate_deployment_shape(&defaulted, "kv-node-0", "attached-ns").is_ok());
    }

    #[test]
    fn deployment_shape_rejects_scaled_out_workload() {
        let scaled = deployment_with_replicas(Some(3));

        let error = validate_deployment_shape(&scaled, "kv-node-0", "attached-ns")
            .expect_err("a multi-replica deployment must be rejected");

        assert!(matches!(
            error,
            K8sAttachedControlError::UnexpectedReplicas { replicas: 3, .. }
        ));
        let message = error.to_string();
        assert!(message.contains("kv-node-0"), "got: {message}");
        assert!(message.contains("attached-ns"), "got: {message}");
    }

    #[tokio::test]
    async fn restart_node_rejects_unknown_node_names() {
        let control = offline_control(AttachedAccess::Direct);

        let error = NodeControl::restart_node(&control, "other-node")
            .await
            .expect_err("an undiscovered node must be rejected");

        let message = error.to_string();
        assert!(
            message.contains("not part of the attached cluster"),
            "got: {message}"
        );
        assert!(message.contains("attached-ns"), "got: {message}");
        assert!(message.contains("kv-node-0"), "got: {message}");
    }

    #[tokio::test]
    async fn restart_node_surfaces_deployment_resolution_errors() {
        let control = offline_control(AttachedAccess::Direct);

        let error = NodeControl::restart_node(&control, "kv-node-0")
            .await
            .expect_err("offline restart must fail against the unreachable API server");

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "restart_node must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("kv-node-0") && message.contains("attached-ns"),
            "the error must name the deployment and namespace: {message}"
        );
    }

    #[tokio::test]
    async fn restart_node_with_rejects_unsupported_options() {
        let control = offline_control(AttachedAccess::Direct);
        let options = StartNodeOptions::<DummyEnv>::default().with_args(["--flag".to_owned()]);

        let error = NodeControlHandle::restart_node_with_config(&control, "kv-node-0", options)
            .await
            .expect_err("extra process arguments must be rejected on attached restart");

        assert!(
            error.to_string().contains("not supported on restart"),
            "got: {error}"
        );
    }

    #[tokio::test]
    async fn stop_node_surfaces_deployment_resolution_errors() {
        let control = offline_control(AttachedAccess::Direct);

        let error = NodeControl::stop_node(&control, "kv-node-0")
            .await
            .expect_err("offline stop must fail against the unreachable API server");

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "stop_node must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("kv-node-0") && message.contains("attached-ns"),
            "the error must name the deployment and namespace: {message}"
        );
    }

    #[tokio::test]
    async fn start_node_surfaces_deployment_resolution_errors() {
        let control = offline_control(AttachedAccess::Direct);

        let error = expect_start_error(
            NodeControl::start_node(&control, "kv-node-0").await,
            "offline start must fail against the unreachable API server",
        );

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "start_node must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("kv-node-0") && message.contains("attached-ns"),
            "the error must name the deployment and namespace: {message}"
        );
    }

    #[tokio::test]
    async fn start_node_with_rejects_unsupported_options() {
        let control = offline_control(AttachedAccess::Direct);
        let options = StartNodeOptions::<DummyEnv>::default()
            .with_persist_dir(std::path::PathBuf::from("/tmp/demo"));

        let error = expect_start_error(
            NodeControlHandle::start_node_with_config(&control, "kv-node-0", options).await,
            "persist directories must be rejected on attached start",
        );

        assert!(
            error
                .to_string()
                .contains("persist/snapshot directories are not supported"),
            "got: {error}"
        );
    }

    #[tokio::test]
    async fn start_node_with_default_options_surfaces_backend_errors() {
        let control = offline_control(AttachedAccess::Direct);

        let error = expect_start_error(
            NodeControlHandle::start_node_with_config(
                &control,
                "kv-node-0",
                StartNodeOptions::<DummyEnv>::default(),
            )
            .await,
            "offline start must fail against the unreachable API server",
        );

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "start_node_with must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("kv-node-0") && message.contains("attached-ns"),
            "the error must name the deployment and namespace: {message}"
        );
    }

    #[test]
    fn deployment_shape_accepts_stopped_workload() {
        let stopped = deployment_with_replicas(Some(0));

        assert!(
            validate_deployment_shape(&stopped, "kv-node-0", "attached-ns").is_ok(),
            "a scaled-to-zero deployment must stay controllable so start_node can scale it back"
        );
    }

    #[tokio::test]
    async fn wait_node_ready_requires_registered_forward() {
        let control = offline_control(AttachedAccess::Forwarded {
            forwards: AttachedForwardRegistry::default(),
        });

        let error = NodeControl::wait_node_ready(&control, "kv-node-0")
            .await
            .expect_err("a missing forward must fail the readiness wait");

        let message = error.to_string();
        assert!(
            !message.contains("not supported by this deployer"),
            "wait_node_ready must not fall back to the trait default: {message}"
        );
        assert!(
            message.contains("no registered port-forward"),
            "got: {message}"
        );
    }

    #[tokio::test]
    async fn common_control_preserves_attached_validation_errors() {
        let attached = offline_control(AttachedAccess::Forwarded {
            forwards: AttachedForwardRegistry::default(),
        });
        let control: &dyn NodeControl = &attached;
        let options = NodeLaunchOptions::default().with_args(["--flag"]);

        let error = control
            .start_node_with("kv-node-0", options.clone())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not supported on restart"));
        let error = control
            .restart_node_with("kv-node-0", options)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not supported on restart"));
        let error = control.node_access("kv-node-0").await.unwrap_err();
        assert!(error.to_string().contains("no registered port-forward"));
        let error = control.node_access("unknown").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not part of the attached cluster")
        );
        assert_eq!(control.node_pid("kv-node-0"), None);
    }

    #[tokio::test]
    async fn common_control_resolves_attached_service_node_port() {
        use std::{
            io::{Read as _, Write as _},
            net::TcpListener,
            thread,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            let body = serde_json::json!({
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": { "name": "kv-node-0" },
                "spec": {
                    "ports": [{ "name": "http", "port": 8080, "nodePort": 31080 }]
                }
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let mut attached = offline_control(AttachedAccess::Direct);
        attached.client =
            Client::try_from(Config::new(format!("http://{address}").parse().unwrap())).unwrap();
        let control: &dyn NodeControl = &attached;

        let access: NodeAccess = control.node_access("kv-node-0").await.unwrap();

        assert_eq!(access.host(), crate::host::node_host());
        assert_eq!(access.api_port(), 31080);
        assert_eq!(access.testing_port(), None);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn node_names_and_clients_come_from_discovery() {
        let control = offline_control(AttachedAccess::Direct);

        assert_eq!(
            NodeControl::node_names(&control),
            vec!["kv-node-0".to_owned()]
        );
        assert_eq!(
            NodeControlHandle::node_client(&control, "kv-node-0").as_deref(),
            Some("http://kv-node-0/")
        );
        assert!(NodeControlHandle::node_client(&control, "other-node").is_none());
    }
}
