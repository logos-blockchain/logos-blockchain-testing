use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use reqwest::Url;
use testing_framework_core::scenario::{
    CleanupGuard, ClusterControlProfile, ClusterControlRequest, ClusterProvisioner, ClusterRequest,
    ClusterSource, ClusterStartMode, ClusterUnit, ClusterWaitHandle, DynError, ExistingCluster,
    ExternalNodeSource, HttpReadinessRequirement, NodeClients, ObservabilityInputs, RetryPolicy,
};
use tokio_retry::{
    RetryIf,
    strategy::{ExponentialBackoff, jitter},
};
use tracing::{info, warn};

use self::{
    attach_provider::{ComposeAttachProvider, ComposeAttachedClusterWait},
    setup::DeploymentSetup,
};
use crate::{
    ComposeProvisioner,
    container_stack::is_valid_dns_label,
    docker::control::{ComposeAttachedNodeControl, ComposeNodeControl},
    env::{
        ComposeDeployEnv, compose_descriptor, node_container_ports,
        wait_remote_readiness as remote_readiness_future,
    },
    errors::{ComposeRunnerError, ConfigError, StackReadinessError},
    infrastructure::{
        environment::StackEnvironment,
        ports::{
            HostPortMapping, compose_runner_host, discover_host_ports, with_service_namespace,
        },
        project::ComposeProject,
    },
    lifecycle::{
        cleanup::{ParticipantCleanup, preserve_requested},
        readiness::{
            build_node_clients_with_ports, ensure_nodes_ready_with_ports,
            maybe_sleep_for_disabled_readiness,
        },
    },
    session::{ClusterKey, ParticipantId},
};

mod attach_provider;
mod setup;

const PRINT_ENDPOINTS_ENV: &str = "TESTNET_PRINT_ENDPOINTS";

#[async_trait]
impl<E: ComposeDeployEnv> ClusterProvisioner<E> for ComposeProvisioner {
    async fn provision_cluster(
        &self,
        request: ClusterRequest<E>,
    ) -> Result<ClusterUnit<E>, DynError> {
        match request.source().clone() {
            ClusterSource::Managed {
                deployment,
                external,
            } => {
                if request.start_mode() == ClusterStartMode::OnDemand {
                    return Err(provisioner_error(ComposeRunnerError::OnDemandUnsupported));
                }
                provision_managed::<E>(self, &request, &deployment, &external)
                    .await
                    .map_err(provisioner_error)
            }
            ClusterSource::Attached { cluster, external } => {
                provision_attached::<E>(&request, &cluster, &external)
                    .await
                    .map_err(provisioner_error)
            }
            ClusterSource::External { nodes } => {
                provision_external::<E>(&nodes).map_err(provisioner_error)
            }
        }
    }
}

fn provisioner_error(error: ComposeRunnerError) -> DynError {
    Box::new(error)
}

async fn provision_managed<E: ComposeDeployEnv>(
    provisioner: &ComposeProvisioner,
    request: &ClusterRequest<E>,
    deployment: &E::Deployment,
    external: &[ExternalNodeSource],
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let observability = resolve_request_observability(request.observability())?;
    let key = cluster_key(request.name())?;
    let Some(retry_policy) = request.policy().retry_policy else {
        return provision_managed_attempt::<E>(
            provisioner,
            &key,
            request,
            deployment,
            external,
            &observability,
        )
        .await;
    };

    provision_managed_with_retry::<E>(
        provisioner,
        &key,
        request,
        deployment,
        external,
        &observability,
        retry_policy,
    )
    .await
}

fn cluster_key(name: Option<&str>) -> Result<ClusterKey, ComposeRunnerError> {
    const MAX_CLUSTER_NAME_LEN: usize = 32;
    match name {
        None => Ok(ClusterKey::Unnamed),
        Some(name) if is_valid_dns_label(name, MAX_CLUSTER_NAME_LEN) => {
            Ok(ClusterKey::Named(name.to_owned()))
        }
        Some(name) => Err(ComposeRunnerError::InvalidClusterName {
            name: name.to_owned(),
        }),
    }
}

async fn provision_managed_with_retry<E: ComposeDeployEnv>(
    provisioner: &ComposeProvisioner,
    key: &ClusterKey,
    request: &ClusterRequest<E>,
    deployment: &E::Deployment,
    external: &[ExternalNodeSource],
    observability: &ObservabilityInputs,
    retry_policy: RetryPolicy,
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let max_attempts = retry_policy.max_attempts.max(1);
    let attempts = Arc::new(AtomicUsize::new(0));
    let strategy = ExponentialBackoff::from_millis(retry_policy.base_delay.as_millis() as u64)
        .max_delay(retry_policy.max_delay)
        .map(jitter)
        .take(max_attempts.saturating_sub(1));
    let operation = {
        let attempts = Arc::clone(&attempts);
        move || {
            let attempts = Arc::clone(&attempts);
            async move {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
                info!(attempt, max_attempts, "provisioning compose cluster");
                provision_managed_attempt::<E>(
                    provisioner,
                    key,
                    request,
                    deployment,
                    external,
                    observability,
                )
                .await
            }
        }
    };
    let should_retry = {
        let attempts = Arc::clone(&attempts);
        move |error: &ComposeRunnerError| {
            if error.is_terminal() {
                warn!(
                    error = %error,
                    "compose provisioning failed with a terminal error; not retrying"
                );
                return false;
            }
            let attempt = attempts.load(Ordering::Relaxed);
            if attempt < max_attempts {
                warn!(
                    attempt,
                    max_attempts,
                    error = %error,
                    "compose provisioning failed; retrying with backoff"
                );
                true
            } else {
                false
            }
        }
    };

    RetryIf::start(strategy, operation, should_retry).await
}

/// Provisions one managed cluster as its own compose project attached to the
/// provisioner's shared session network.
///
/// The cluster registers its key and service names before any docker work, so
/// concurrent participants can never collide; a failed attempt tears down and
/// deregisters only its own project.
async fn provision_managed_attempt<E: ComposeDeployEnv>(
    provisioner: &ComposeProvisioner,
    key: &ClusterKey,
    request: &ClusterRequest<E>,
    deployment: &E::Deployment,
    external: &[ExternalNodeSource],
    observability: &ObservabilityInputs,
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let namespace = key.namespace().map(str::to_owned);
    with_service_namespace(namespace, async {
        let setup = DeploymentSetup::<E>::new(deployment);
        setup.validate_environment().await?;

        let participant = provisioner
            .inner
            .register_cluster(key, planned_service_names::<E>(deployment)?)?;

        deploy_registered_cluster::<E>(
            provisioner,
            setup,
            participant,
            key,
            request,
            deployment,
            external,
            observability,
        )
        .await
    })
    .await
}

/// Returns the compose service names the deployment will provision, derived
/// from the same descriptor the workspace preparation renders.
fn planned_service_names<E: ComposeDeployEnv>(
    deployment: &E::Deployment,
) -> Result<Vec<String>, ComposeRunnerError> {
    let descriptor = compose_descriptor::<E>(deployment, 0)
        .map_err(|source| ComposeRunnerError::Config(ConfigError::Descriptor { source }))?;
    Ok(descriptor
        .nodes()
        .iter()
        .map(|node| node.name().to_owned())
        .collect())
}

#[expect(
    clippy::too_many_arguments,
    reason = "cluster deployment needs the full provisioning context"
)]
async fn deploy_registered_cluster<E: ComposeDeployEnv>(
    provisioner: &ComposeProvisioner,
    setup: DeploymentSetup<'_, E>,
    participant: ParticipantId,
    key: &ClusterKey,
    request: &ClusterRequest<E>,
    deployment: &E::Deployment,
    external: &[ExternalNodeSource],
    observability: &ObservabilityInputs,
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let policy = request.policy();

    if let Err(error) = provisioner.inner.network().ensure_created().await {
        release_failed_participant(provisioner, participant).await;
        return Err(error);
    }

    let mut environment = match setup
        .prepare_workspace(
            observability,
            &key.cfgsync_file_name(),
            Some(provisioner.inner.network().name()),
        )
        .await
    {
        Ok(environment) => environment,
        Err(error) => {
            release_failed_participant(provisioner, participant).await;
            return Err(error);
        }
    };

    let project = environment.project().clone();
    let service_names: Vec<String> = environment
        .descriptor()
        .nodes()
        .iter()
        .map(|node| node.name().to_owned())
        .collect();

    let deployed = match resolve_cluster_nodes::<E>(
        &project,
        deployment,
        policy.readiness_enabled,
        policy.readiness_requirement,
    )
    .await
    {
        Ok(deployed) => deployed,
        Err(error) => {
            fail_cluster_environment(
                provisioner,
                participant,
                &mut environment,
                policy.cleanup_policy.preserve_artifacts,
                "compose cluster runtime resolution failed",
            )
            .await;
            return Err(error);
        }
    };

    if let Err(error) = append_external_clients::<E>(&deployed.node_clients, external) {
        fail_cluster_environment(
            provisioner,
            participant,
            &mut environment,
            policy.cleanup_policy.preserve_artifacts,
            "failed to build external node clients",
        )
        .await;
        return Err(error);
    }

    log_observability_endpoints(observability);
    log_profiling_urls(&deployed.host, &deployed.host_ports);
    maybe_print_endpoints(observability, &deployed.host, &deployed.host_ports);

    if policy.cleanup_policy.preserve_artifacts {
        provisioner.inner.preserve().request();
    }
    let cleanup = match environment.into_cleanup() {
        Ok(cleanup) => cleanup
            .with_preserve_artifacts(policy.cleanup_policy.preserve_artifacts)
            .with_session_preservation(provisioner.inner.preserve().clone()),
        Err(error) => {
            release_failed_participant(provisioner, participant).await;
            return Err(error);
        }
    };
    let guard = ParticipantCleanup::new(Arc::clone(&provisioner.inner), participant, cleanup);

    let node_names = cluster_node_names(service_names, deployed.host_ports.nodes.len());
    Ok(managed_cluster_unit::<E>(
        request,
        deployment,
        observability,
        &project,
        node_names,
        deployed,
        Box::new(guard),
    ))
}

/// Applies the preservation policy to a failed attempt: when preservation is
/// requested — by the environment, any session participant, or the failing
/// request's own cleanup policy — the attempt's containers keep running, its
/// workspace and cfgsync are preserved, and its registration stays in place;
/// otherwise the attempt tears down and deregisters only its own project.
async fn fail_cluster_environment(
    provisioner: &ComposeProvisioner,
    participant: ParticipantId,
    environment: &mut StackEnvironment,
    policy_preserve: bool,
    reason: &str,
) {
    let shared = provisioner.inner.preserve();
    if policy_preserve || preserve_requested() || shared.requested() {
        shared.request();
        environment.fail_preserving(reason).await;
        return;
    }
    environment.fail(reason).await;
    release_failed_participant(provisioner, participant).await;
}

async fn release_failed_participant(provisioner: &ComposeProvisioner, participant: ParticipantId) {
    provisioner.inner.deregister(participant);
    provisioner.inner.release_network_if_unused().await;
}

fn cluster_node_names(service_names: Vec<String>, node_count: usize) -> Vec<String> {
    let mut names = service_names;
    names.truncate(node_count);
    names
}

fn managed_cluster_unit<E: ComposeDeployEnv>(
    request: &ClusterRequest<E>,
    deployment: &E::Deployment,
    observability: &ObservabilityInputs,
    project: &ComposeProject,
    node_names: Vec<String>,
    deployed: DeployedNodes<E>,
    cleanup: Box<dyn CleanupGuard>,
) -> ClusterUnit<E> {
    let cluster_wait = ComposeManagedClusterWait::<E> {
        deployment: deployment.clone(),
        host_ports: deployed.host_ports.clone(),
    };
    let attachment =
        ExistingCluster::for_compose_services(project.name().to_owned(), node_names.clone());

    let mut unit = ClusterUnit::new(
        Some(deployment.clone()),
        deployed.node_clients,
        ClusterControlProfile::FrameworkManaged,
    )
    .with_cluster_wait(Arc::new(cluster_wait))
    .with_cleanup(cleanup)
    .with_observability(observability.clone())
    .with_attachment(attachment);

    if request.control() == ClusterControlRequest::Full {
        unit = unit.with_node_control(Arc::new(ComposeNodeControl {
            project: project.clone(),
            node_names,
        }));
    }

    unit
}

async fn provision_attached<E: ComposeDeployEnv>(
    request: &ClusterRequest<E>,
    cluster: &ExistingCluster,
    external: &[ExternalNodeSource],
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let observability = resolve_request_observability(request.observability())?;
    let provider = ComposeAttachProvider::<E>::new(compose_runner_host());
    let attached = provider
        .discover(cluster)
        .await
        .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;

    let node_clients = NodeClients::<E>::default();
    let mut node_names = Vec::with_capacity(attached.len());
    for (service, client) in attached {
        node_names.push(service);
        node_clients.add_node(client);
    }
    append_external_clients::<E>(&node_clients, external)?;

    if node_clients.is_empty() {
        return Err(ComposeRunnerError::RuntimePreflight);
    }

    let cluster_wait = ComposeAttachedClusterWait::<E>::try_new(compose_runner_host(), cluster)
        .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;

    let mut unit = ClusterUnit::new(
        None,
        node_clients,
        ClusterControlProfile::ExistingClusterAttached,
    )
    .with_cluster_wait(Arc::new(cluster_wait))
    .with_observability(observability)
    .with_attachment(cluster.clone());

    if request.control() == ClusterControlRequest::Full {
        let node_control =
            ComposeAttachedNodeControl::try_from_existing_cluster(cluster, node_names)
                .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;
        unit = unit.with_node_control(Arc::new(node_control));
    }

    Ok(unit)
}

fn provision_external<E: ComposeDeployEnv>(
    nodes: &[ExternalNodeSource],
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let node_clients = NodeClients::<E>::default();
    append_external_clients::<E>(&node_clients, nodes)?;

    Ok(ClusterUnit::new(
        None,
        node_clients,
        ClusterControlProfile::ExternalUncontrolled,
    ))
}

fn append_external_clients<E: ComposeDeployEnv>(
    node_clients: &NodeClients<E>,
    sources: &[ExternalNodeSource],
) -> Result<(), ComposeRunnerError> {
    for source in sources {
        let client = E::external_node_client(source)
            .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;
        node_clients.add_node(client);
    }

    Ok(())
}

fn resolve_request_observability(
    overrides: &ObservabilityInputs,
) -> Result<ObservabilityInputs, ComposeRunnerError> {
    Ok(ObservabilityInputs::from_env()?.with_overrides(overrides.clone()))
}

struct ComposeManagedClusterWait<E: ComposeDeployEnv> {
    deployment: E::Deployment,
    host_ports: HostPortMapping,
}

#[async_trait]
impl<E: ComposeDeployEnv> ClusterWaitHandle<E> for ComposeManagedClusterWait<E> {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        E::wait_remote_readiness(
            &self.deployment,
            &self.host_ports,
            HttpReadinessRequirement::AllNodesReady,
        )
        .await
    }
}

pub(crate) struct DeployedNodes<E: ComposeDeployEnv> {
    pub(crate) host_ports: HostPortMapping,
    pub(crate) host: String,
    pub(crate) node_clients: NodeClients<E>,
}

/// Discovers host ports, applies the readiness policy, and builds node clients
/// for a running cluster inside the given Compose project.
pub(crate) async fn resolve_cluster_nodes<E: ComposeDeployEnv>(
    project: &ComposeProject,
    descriptors: &E::Deployment,
    readiness_enabled: bool,
    readiness_requirement: HttpReadinessRequirement,
) -> Result<DeployedNodes<E>, ComposeRunnerError> {
    let nodes = node_container_ports::<E>(descriptors)
        .map_err(|source| ComposeRunnerError::Config(ConfigError::Descriptor { source }))?;
    let host_ports = discover_host_ports(project, &nodes).await?;

    if readiness_enabled {
        let node_ports = host_ports.node_api_ports();
        info!(ports = ?node_ports, "waiting for node HTTP endpoints");
        ensure_nodes_ready_with_ports::<E>(&node_ports, readiness_requirement).await?;

        info!("waiting for remote service readiness");
        remote_readiness_future::<E>(descriptors, &host_ports, readiness_requirement)
            .map_err(|source| {
                ComposeRunnerError::Readiness(StackReadinessError::Remote { source })
            })?
            .await
            .map_err(|source| {
                ComposeRunnerError::Readiness(StackReadinessError::Remote { source })
            })?;

        info!("compose readiness checks passed");
    } else {
        info!("readiness checks disabled; giving the stack a short grace period");
        maybe_sleep_for_disabled_readiness(false).await;
    }

    let host = compose_runner_host();
    let node_clients = build_node_clients_with_ports::<E>(descriptors, &host_ports, &host)?;

    Ok(DeployedNodes {
        host_ports,
        host,
        node_clients,
    })
}

pub(crate) fn log_observability_endpoints(observability: &ObservabilityInputs) {
    if let Some(url) = observability.metrics_query_url.as_ref() {
        info!(
            metrics_query_url = %url.as_str(),
            "metrics query endpoint configured"
        );
    }

    if let Some(url) = observability.grafana_url.as_ref() {
        info!(grafana_url = %url.as_str(), "grafana url configured");
    }
}

pub(crate) fn maybe_print_endpoints(
    observability: &ObservabilityInputs,
    host: &str,
    ports: &HostPortMapping,
) {
    if !should_print_endpoints() {
        return;
    }

    let prometheus = endpoint_or_disabled(observability.metrics_query_url.as_ref());
    let grafana = endpoint_or_disabled(observability.grafana_url.as_ref());

    println!(
        "TESTNET_ENDPOINTS prometheus={} grafana={}",
        prometheus, grafana
    );

    print_profiling_urls(host, ports);
}

fn should_print_endpoints() -> bool {
    env::var(PRINT_ENDPOINTS_ENV).is_ok()
}

fn endpoint_or_disabled(endpoint: Option<&Url>) -> String {
    endpoint.map_or_else(|| "<disabled>".to_string(), |url| url.as_str().to_string())
}

pub(crate) fn log_profiling_urls(host: &str, ports: &HostPortMapping) {
    for (idx, node) in ports.nodes.iter().enumerate() {
        info!(
            node = idx,
            profiling_url = %profiling_url(host, node.api),
            "node profiling endpoint (profiling feature required)"
        );
    }
}

fn print_profiling_urls(host: &str, ports: &HostPortMapping) {
    for (idx, node) in ports.nodes.iter().enumerate() {
        println!(
            "TESTNET_PPROF node_{}={}",
            idx,
            profiling_url(host, node.api)
        );
    }
}

fn profiling_url(host: &str, api_port: u16) -> String {
    format!("http://{host}:{api_port}/debug/pprof/profile?seconds=15&format=proto")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use testing_framework_core::{
        scenario::{
            CleanupPolicy, ClusterProvisioner as _, ClusterRequest, ClusterStartMode,
            DeploymentPolicy, DynError, ExternalNodeSource, RetryPolicy,
        },
        topology::DeploymentDescriptor,
    };

    use super::{cluster_key, cluster_node_names, provisioner_error};
    use crate::{
        ComposeProvisioner,
        descriptor::{ComposeDescriptor, NodeDescriptor},
        env::ComposeDeployEnv,
        errors::ComposeRunnerError,
        infrastructure::ports::node_identifier,
        session::ClusterKey,
    };

    #[derive(Clone)]
    struct TestTopology;

    impl DeploymentDescriptor for TestTopology {
        fn node_count(&self) -> usize {
            1
        }
    }

    struct TestEnv;

    #[async_trait::async_trait]
    impl testing_framework_core::scenario::Application for TestEnv {
        type Deployment = TestTopology;
        type NodeClient = String;
        type NodeConfig = ();

        fn external_node_client(source: &ExternalNodeSource) -> Result<Self::NodeClient, DynError> {
            Ok(source.endpoint().to_owned())
        }
    }

    #[async_trait::async_trait]
    impl ComposeDeployEnv for TestEnv {
        fn compose_descriptor(
            topology: &TestTopology,
            _cfgsync_port: u16,
        ) -> Result<ComposeDescriptor, DynError> {
            let nodes = (0..topology.node_count())
                .map(|index| {
                    NodeDescriptor::with_loopback_ports(
                        node_identifier(index),
                        "cluster-node:local",
                        vec!["/bin/node".to_owned()],
                        Vec::new(),
                        Vec::new(),
                        vec![8080],
                        Vec::new(),
                        None,
                    )
                })
                .collect();
            Ok(ComposeDescriptor::new(nodes))
        }
    }

    fn named_key(name: &str) -> ClusterKey {
        ClusterKey::Named(name.to_owned())
    }

    #[tokio::test]
    async fn on_demand_start_is_rejected() {
        let request = ClusterRequest::<TestEnv>::managed(TestTopology)
            .with_start_mode(ClusterStartMode::OnDemand);

        let error = ComposeProvisioner::default()
            .provision_cluster(request)
            .await
            .err()
            .expect("on-demand start must be rejected");

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::OnDemandUnsupported)
        ));
        assert_eq!(
            error.to_string(),
            "compose provisioner does not support on-demand start"
        );
    }

    #[tokio::test]
    async fn external_unit_is_borrowed_and_uncontrolled() {
        let source = ExternalNodeSource::new("external-0".into(), "http://node-0".into());

        let mut unit = ComposeProvisioner::default()
            .provision_cluster(ClusterRequest::<TestEnv>::external(vec![source]))
            .await
            .expect("external unit should resolve");

        assert_eq!(unit.node_clients().snapshot(), vec!["http://node-0"]);
        assert!(unit.node_control().is_none());
        assert!(unit.cluster_wait().is_none());
        assert!(unit.take_cleanup().is_none());
    }

    #[test]
    fn docker_unavailable_stays_downcastable() {
        let error = provisioner_error(ComposeRunnerError::DockerUnavailable);

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::DockerUnavailable)
        ));
    }

    #[tokio::test]
    async fn a_second_unnamed_cluster_is_rejected_with_naming_guidance() {
        let provisioner = ComposeProvisioner::default();
        provisioner
            .inner
            .register_cluster(&ClusterKey::Unnamed, vec!["node-0".to_owned()])
            .expect("first unnamed cluster must register");

        let error = provisioner
            .provision_cluster(ClusterRequest::<TestEnv>::managed(TestTopology))
            .await
            .err()
            .expect("second unnamed cluster must be rejected");

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::UnnamedClusterAlreadyProvisioned)
        ));
        assert!(error.to_string().contains("with_name"));
    }

    #[tokio::test]
    async fn duplicate_named_cluster_is_rejected_through_the_provisioner() {
        let provisioner = ComposeProvisioner::default();
        provisioner
            .inner
            .register_cluster(&named_key("alpha"), vec!["alpha-node-0".to_owned()])
            .expect("first alpha cluster must register");

        let error = provisioner
            .provision_cluster(ClusterRequest::<TestEnv>::managed(TestTopology).with_name("alpha"))
            .await
            .err()
            .expect("duplicate named cluster must be rejected");

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::ClusterAlreadyProvisioned { name }) if name == "alpha"
        ));
    }

    #[tokio::test]
    async fn conflicting_service_names_are_rejected_through_the_provisioner() {
        let provisioner = ComposeProvisioner::default();
        provisioner
            .inner
            .register_stack(vec!["node-0".to_owned()])
            .expect("stack must register");

        let error = provisioner
            .provision_cluster(ClusterRequest::<TestEnv>::managed(TestTopology))
            .await
            .err()
            .expect("colliding service names must be rejected");

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::ServiceNameConflict { name }) if name == "node-0"
        ));
    }

    #[tokio::test]
    async fn invalid_cluster_names_are_rejected() {
        let error = ComposeProvisioner::default()
            .provision_cluster(
                ClusterRequest::<TestEnv>::managed(TestTopology).with_name("Bad_Name"),
            )
            .await
            .err()
            .expect("invalid cluster names must be rejected");

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::InvalidClusterName { name }) if name == "Bad_Name"
        ));
    }

    #[test]
    fn cluster_keys_carry_the_service_namespace() {
        assert_eq!(cluster_key(None).unwrap(), ClusterKey::Unnamed);
        assert_eq!(
            cluster_key(Some("alpha")).unwrap().namespace(),
            Some("alpha")
        );
        assert!(cluster_key(Some("UPPER")).is_err());
        assert!(cluster_key(Some("-edge")).is_err());
        assert!(cluster_key(Some("")).is_err());
    }

    #[test]
    fn node_names_exclude_extra_services() {
        let names = cluster_node_names(vec!["node-0".to_owned(), "sidecar".to_owned()], 1);

        assert_eq!(names, ["node-0"]);
    }

    #[tokio::test]
    async fn a_duplicate_cluster_fails_fast_without_burning_retries() {
        let provisioner = ComposeProvisioner::default();
        provisioner
            .inner
            .register_cluster(&named_key("alpha"), vec!["alpha-node-0".to_owned()])
            .expect("first alpha cluster must register");

        let policy = DeploymentPolicy {
            retry_policy: Some(RetryPolicy::new(
                3,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )),
            cleanup_policy: CleanupPolicy::new(false),
            ..DeploymentPolicy::default()
        };
        let request = ClusterRequest::<TestEnv>::managed(TestTopology)
            .with_name("alpha")
            .with_policy(policy);

        let error = tokio::time::timeout(
            Duration::from_secs(5),
            provisioner.provision_cluster(request),
        )
        .await
        .expect("terminal errors must not wait out retry backoff")
        .err()
        .expect("duplicate cluster must be rejected");

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::ClusterAlreadyProvisioned { name }) if name == "alpha"
        ));
    }

    #[test]
    fn terminal_errors_are_not_retryable() {
        assert!(
            ComposeRunnerError::ClusterAlreadyProvisioned {
                name: "alpha".to_owned()
            }
            .is_terminal()
        );
        assert!(ComposeRunnerError::UnnamedClusterAlreadyProvisioned.is_terminal());
        assert!(
            ComposeRunnerError::ServiceNameConflict {
                name: "node-0".to_owned()
            }
            .is_terminal()
        );
        assert!(
            ComposeRunnerError::InvalidClusterName {
                name: "Bad_Name".to_owned()
            }
            .is_terminal()
        );
        assert!(ComposeRunnerError::OnDemandUnsupported.is_terminal());
        assert!(!ComposeRunnerError::DockerUnavailable.is_terminal());
        assert!(!ComposeRunnerError::RuntimePreflight.is_terminal());
    }
}
