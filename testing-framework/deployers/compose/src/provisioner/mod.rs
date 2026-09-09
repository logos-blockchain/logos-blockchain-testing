use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::PathBuf,
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
use tokio::sync::MutexGuard;
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
        ComposeDeployEnv, ConfigServerHandle, compose_descriptor, node_container_ports,
        wait_remote_readiness as remote_readiness_future,
    },
    errors::{ComposeRunnerError, ConfigError, StackReadinessError},
    infrastructure::{
        environment::{
            allocate_cfgsync_port, ensure_compose_images_present, start_cfgsync_stage,
            update_cfgsync_logged,
        },
        ports::{
            HostPortMapping, compose_runner_host, discover_host_ports, with_service_namespace,
        },
        project::ComposeProject,
        template::write_compose_file,
    },
    lifecycle::{
        cleanup::{ClusterServicesCleanup, preserve_requested},
        readiness::{
            build_node_clients_with_ports, ensure_nodes_ready_with_ports,
            maybe_sleep_for_disabled_readiness,
        },
    },
    session::{
        ClusterKey, ClusterServices, ComposeSession, ComposeSessionCleanup, RunnerPorts,
        SessionPreservation, session_descriptor,
    },
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

        let mutation = provisioner.inner.mutation.lock().await;
        let session = provisioner.session_snapshot();
        ensure_session_accepts_cluster(session.as_ref(), key)?;

        match session {
            Some(session) => {
                extend_shared_session::<E>(
                    provisioner,
                    mutation,
                    session,
                    key,
                    request,
                    deployment,
                    external,
                    observability,
                )
                .await
            }
            None => {
                start_shared_session::<E>(
                    provisioner,
                    mutation,
                    setup,
                    key,
                    request,
                    deployment,
                    external,
                    observability,
                )
                .await
            }
        }
    })
    .await
}

fn ensure_session_accepts_cluster(
    session: Option<&ComposeSession>,
    key: &ClusterKey,
) -> Result<(), ComposeRunnerError> {
    let Some(session) = session else {
        return Ok(());
    };
    if session.poisoned {
        return Err(ComposeRunnerError::SessionPoisoned);
    }
    if session.clusters.contains_key(key) {
        return Err(match key {
            ClusterKey::Unnamed => ComposeRunnerError::UnnamedClusterAlreadyProvisioned,
            ClusterKey::Named(name) => {
                ComposeRunnerError::ClusterAlreadyProvisioned { name: name.clone() }
            }
        });
    }
    Ok(())
}

/// Establishes the shared Compose session with the cluster as its first
/// resource; the returned cleanup guard owns whole-project teardown.
///
/// The provisioner mutation lock is held for the whole attempt, readiness
/// included, and the session is stored only once the cluster is up — so no
/// other participant can ever observe a half-provisioned session.
#[expect(
    clippy::too_many_arguments,
    reason = "session establishment needs the full provisioning context"
)]
async fn start_shared_session<E: ComposeDeployEnv>(
    provisioner: &ComposeProvisioner,
    mutation: MutexGuard<'_, ()>,
    setup: DeploymentSetup<'_, E>,
    key: &ClusterKey,
    request: &ClusterRequest<E>,
    deployment: &E::Deployment,
    external: &[ExternalNodeSource],
    observability: &ObservabilityInputs,
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let policy = request.policy();
    let environment = setup
        .prepare_workspace(observability, &key.cfgsync_file_name())
        .await?;

    let project = environment.project().clone();
    let cluster = ClusterServices::new(environment.descriptor().nodes().to_vec());
    let generation = provisioner.next_generation();
    let preserve = SessionPreservation::default();
    if policy.cleanup_policy.preserve_artifacts {
        preserve.request();
    }

    let mut environment = environment;
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
            environment
                .fail("compose cluster runtime resolution failed")
                .await;
            return Err(error);
        }
    };

    if let Err(error) = append_external_clients::<E>(&deployed.node_clients, external) {
        environment
            .fail("failed to build external node clients")
            .await;
        return Err(error);
    }

    log_observability_endpoints(observability);
    log_profiling_urls(&deployed.host, &deployed.host_ports);
    maybe_print_endpoints(observability, &deployed.host, &deployed.host_ports);

    let cleanup = environment
        .into_cleanup()?
        .with_preserve_artifacts(policy.cleanup_policy.preserve_artifacts)
        .with_session_preservation(preserve.clone());
    provisioner.store_session(ComposeSession {
        project: project.clone(),
        services: Vec::new(),
        runner_ports: RunnerPorts::default(),
        clusters: BTreeMap::from([(key.clone(), cluster.clone())]),
        poisoned: false,
        generation,
        preserve,
    });
    drop(mutation);
    let guard = ComposeSessionCleanup {
        inner: Arc::clone(&provisioner.inner),
        cleanup: Some(cleanup),
        generation,
    };
    let node_names = cluster_node_names(&cluster, deployed.host_ports.nodes.len());

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
/// Extends an established shared session with the cluster's services; failed
/// extensions roll back without touching running container services.
///
/// The caller-held provisioner mutation lock is held for the whole attempt,
/// readiness included, and the extension is committed to the session model
/// only once the cluster is up.
#[expect(
    clippy::too_many_arguments,
    reason = "session extension needs the full provisioning context"
)]
async fn extend_shared_session<E: ComposeDeployEnv>(
    provisioner: &ComposeProvisioner,
    mutation: MutexGuard<'_, ()>,
    session: ComposeSession,
    key: &ClusterKey,
    request: &ClusterRequest<E>,
    deployment: &E::Deployment,
    external: &[ExternalNodeSource],
    observability: &ObservabilityInputs,
) -> Result<ClusterUnit<E>, ComposeRunnerError> {
    let policy = request.policy();
    let cfgsync_port = allocate_cfgsync_port()?;
    let cfgsync_path = session_cfgsync_path(&session.project, key);
    update_cfgsync_logged::<E>(
        &cfgsync_path,
        deployment,
        cfgsync_port,
        observability.metrics_otlp_ingest_url.as_ref(),
    )?;
    let descriptor = compose_descriptor::<E>(deployment, cfgsync_port)
        .map_err(|source| ComposeRunnerError::Config(ConfigError::Descriptor { source }))?;
    let cluster = ClusterServices::new(descriptor.nodes().to_vec());
    ensure_cluster_names_available(&session, &cluster)?;
    ensure_compose_images_present::<E>(&descriptor, &cfgsync_path, cfgsync_port).await?;
    let mut clusters = session.clusters.clone();
    clusters.insert(key.clone(), cluster.clone());
    let merged = session_descriptor(&clusters, &session.services, &session.runner_ports)
        .map_err(|source| ComposeRunnerError::Config(ConfigError::Descriptor { source }))?;
    let service_names = cluster
        .service_names()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let project_access = session.project.lock_mutation().await;
    if let Err(source) = write_compose_file(&merged, session.project.compose_file()) {
        let restore = restore_session_compose_file(&session);
        drop(project_access);
        let failure = ComposeRunnerError::Config(ConfigError::Template { source });
        if let Err(rollback) = restore {
            poison_session(provisioner, session.generation);
            return Err(ComposeRunnerError::ExtensionRollback {
                failure: Box::new(failure),
                rollback: Box::new(rollback),
            });
        }
        return Err(failure);
    }
    let mut cfgsync_handle =
        match start_cfgsync_stage::<E>(&cfgsync_path, cfgsync_port, session.project.name()).await {
            Ok(handle) => handle,
            Err(failure) => {
                let restore = restore_session_compose_file(&session);
                drop(project_access);
                if let Err(rollback) = restore {
                    poison_session(provisioner, session.generation);
                    return Err(ComposeRunnerError::ExtensionRollback {
                        failure: Box::new(failure),
                        rollback: Box::new(rollback),
                    });
                }
                return Err(failure);
            }
        };
    let up_result = session.project.up_services_unlocked(&service_names).await;
    drop(project_access);
    if let Err(source) = up_result {
        session.project.dump_logs().await;
        let failure = ComposeRunnerError::Compose(source);
        return Err(rolled_back_failure(
            provisioner,
            &session,
            &service_names,
            &mut cfgsync_handle,
            policy.cleanup_policy.preserve_artifacts,
            failure,
        )
        .await);
    }

    let deployed = match resolve_cluster_nodes::<E>(
        &session.project,
        deployment,
        policy.readiness_enabled,
        policy.readiness_requirement,
    )
    .await
    {
        Ok(deployed) => deployed,
        Err(failure) => {
            session.project.dump_logs().await;
            return Err(rolled_back_failure(
                provisioner,
                &session,
                &service_names,
                &mut cfgsync_handle,
                policy.cleanup_policy.preserve_artifacts,
                failure,
            )
            .await);
        }
    };

    if let Err(failure) = append_external_clients::<E>(&deployed.node_clients, external) {
        session.project.dump_logs().await;
        return Err(rolled_back_failure(
            provisioner,
            &session,
            &service_names,
            &mut cfgsync_handle,
            policy.cleanup_policy.preserve_artifacts,
            failure,
        )
        .await);
    }

    let preserve = session.preserve.clone();
    if policy.cleanup_policy.preserve_artifacts {
        preserve.request();
    }
    let generation = session.generation;
    provisioner.store_session(ComposeSession {
        clusters,
        poisoned: false,
        ..session.clone()
    });
    drop(mutation);

    log_observability_endpoints(observability);
    log_profiling_urls(&deployed.host, &deployed.host_ports);
    maybe_print_endpoints(observability, &deployed.host, &deployed.host_ports);

    let node_names = cluster_node_names(&cluster, deployed.host_ports.nodes.len());
    let guard = ClusterServicesCleanup::new(
        Arc::clone(&provisioner.inner),
        session.project.clone(),
        key.clone(),
        service_names,
        cfgsync_handle,
        policy.cleanup_policy.preserve_artifacts,
        preserve,
        generation,
    );

    Ok(managed_cluster_unit::<E>(
        request,
        deployment,
        observability,
        &session.project,
        node_names,
        deployed,
        Box::new(guard),
    ))
}

fn cluster_node_names(cluster: &ClusterServices, node_count: usize) -> Vec<String> {
    cluster
        .service_names()
        .take(node_count)
        .map(str::to_owned)
        .collect()
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

/// Rolls back an extension that has not been committed to the shared session
/// model yet; the caller still holds the provisioner mutation lock.
async fn rolled_back_failure(
    provisioner: &ComposeProvisioner,
    session: &ComposeSession,
    service_names: &[String],
    cfgsync_handle: &mut Option<Box<dyn ConfigServerHandle>>,
    policy_preserve: bool,
    failure: ComposeRunnerError,
) -> ComposeRunnerError {
    if rollback_preserves(provisioner, session, cfgsync_handle, policy_preserve) {
        return failure;
    }

    let rollback = async {
        let _project_access = session.project.lock_mutation().await;
        session
            .project
            .remove_services_unlocked(service_names)
            .await
            .map_err(ComposeRunnerError::Compose)?;
        restore_session_compose_file(session)
    }
    .await;

    finish_rollback(provisioner, session.generation, rollback, failure)
}

/// Applies the preservation policy before any rollback teardown: when
/// preservation is requested — by the environment, any session participant,
/// or the failing request's own cleanup policy — the cfgsync server is
/// marked preserved instead of being shut down and the session is poisoned
/// in place.
fn rollback_preserves(
    provisioner: &ComposeProvisioner,
    session: &ComposeSession,
    cfgsync_handle: &mut Option<Box<dyn ConfigServerHandle>>,
    policy_preserve: bool,
) -> bool {
    if policy_preserve || preserve_requested() || session.preserve.requested() {
        if let Some(handle) = cfgsync_handle.as_deref_mut() {
            handle.mark_preserved();
        }
        poison_session(provisioner, session.generation);
        return true;
    }
    if let Some(handle) = cfgsync_handle.as_deref_mut() {
        handle.shutdown();
    }
    false
}

fn finish_rollback(
    provisioner: &ComposeProvisioner,
    generation: u64,
    rollback: Result<(), ComposeRunnerError>,
    failure: ComposeRunnerError,
) -> ComposeRunnerError {
    match rollback {
        Ok(()) => failure,
        Err(rollback) => {
            poison_session(provisioner, generation);
            ComposeRunnerError::ExtensionRollback {
                failure: Box::new(failure),
                rollback: Box::new(rollback),
            }
        }
    }
}

fn restore_session_compose_file(session: &ComposeSession) -> Result<(), ComposeRunnerError> {
    let restored = session_descriptor(&session.clusters, &session.services, &session.runner_ports)
        .map_err(|source| ComposeRunnerError::Config(ConfigError::Descriptor { source }))?;
    write_compose_file(&restored, session.project.compose_file())
        .map_err(|source| ComposeRunnerError::Config(ConfigError::Template { source }))
}

/// Marks the session with the expected generation as poisoned; a stale
/// request from a dead session leaves an unrelated newer session untouched.
fn poison_session(provisioner: &ComposeProvisioner, expected_generation: u64) {
    let mut slot = provisioner
        .inner
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match slot.as_mut() {
        Some(session) if session.generation == expected_generation => session.poisoned = true,
        Some(session) => warn!(
            expected_generation,
            live_generation = session.generation,
            "stale compose session poison request; leaving the live session untouched"
        ),
        None => {}
    }
}

fn ensure_cluster_names_available(
    session: &ComposeSession,
    cluster: &ClusterServices,
) -> Result<(), ComposeRunnerError> {
    let existing = session.service_names().collect::<BTreeSet<_>>();
    if let Some(name) = cluster.service_names().find(|name| existing.contains(name)) {
        return Err(ComposeRunnerError::ServiceNameConflict {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// Returns the per-cluster cfgsync config path so each cluster's config
/// server state stays isolated within the shared workspace; owners and
/// joiners derive the same path from their own cluster key alone.
fn session_cfgsync_path(project: &ComposeProject, key: &ClusterKey) -> PathBuf {
    project.root().join("stack").join(key.cfgsync_file_name())
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
    use std::{collections::BTreeMap, path::PathBuf, time::Duration};

    use testing_framework_core::{
        scenario::{
            CleanupPolicy, ClusterProvisioner as _, ClusterRequest, ClusterStartMode,
            DeploymentPolicy, DynError, ExternalNodeSource, RetryPolicy,
        },
        topology::DeploymentDescriptor,
    };

    use super::{
        cluster_key, ensure_cluster_names_available, ensure_session_accepts_cluster,
        poison_session, provisioner_error, session_cfgsync_path,
    };
    use crate::{
        ComposeProvisioner,
        descriptor::NodeDescriptor,
        env::ComposeDeployEnv,
        errors::ComposeRunnerError,
        infrastructure::project::ComposeProject,
        session::{ClusterKey, ClusterServices, ComposeSession, RunnerPorts, SessionPreservation},
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
    impl ComposeDeployEnv for TestEnv {}

    fn cluster_node(name: &str) -> NodeDescriptor {
        NodeDescriptor::with_loopback_ports(
            name,
            "cluster-node:local",
            vec!["/bin/node".to_owned()],
            Vec::new(),
            Vec::new(),
            vec![8080],
            Vec::new(),
            None,
        )
    }

    fn session_with_containers(names: &[&str]) -> ComposeSession {
        ComposeSession {
            project: ComposeProject::new(
                PathBuf::from("/tmp/compose.yml"),
                "test-session",
                PathBuf::from("/tmp"),
            ),
            services: names
                .iter()
                .map(|name| {
                    testing_framework_container::ContainerServiceSpec::new(*name, "example:local")
                        .with_command(["/bin/example"])
                })
                .collect(),
            runner_ports: RunnerPorts::new(),
            clusters: BTreeMap::new(),
            poisoned: false,
            generation: 1,
            preserve: SessionPreservation::default(),
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

    #[test]
    fn cluster_names_may_not_collide_with_container_services() {
        let session = session_with_containers(&["worker", "node-1"]);
        let cluster = ClusterServices::new(vec![cluster_node("node-0"), cluster_node("node-1")]);

        let error = ensure_cluster_names_available(&session, &cluster)
            .err()
            .expect("colliding names must be rejected");

        assert!(matches!(
            error,
            ComposeRunnerError::ServiceNameConflict { name } if name == "node-1"
        ));
    }

    #[test]
    fn cluster_names_may_not_collide_with_other_cluster_services() {
        let mut session = session_with_containers(&[]);
        session.clusters.insert(
            named_key("alpha"),
            ClusterServices::new(vec![cluster_node("alpha-node-0")]),
        );
        let cluster = ClusterServices::new(vec![cluster_node("alpha-node-0")]);

        let error = ensure_cluster_names_available(&session, &cluster)
            .err()
            .expect("colliding cluster names must be rejected");

        assert!(matches!(
            error,
            ComposeRunnerError::ServiceNameConflict { name } if name == "alpha-node-0"
        ));
    }

    #[test]
    fn disjoint_cluster_names_extend_the_session() {
        let session = session_with_containers(&["worker"]);
        let cluster = ClusterServices::new(vec![cluster_node("node-0")]);

        assert!(ensure_cluster_names_available(&session, &cluster).is_ok());
    }

    #[test]
    fn two_named_clusters_are_accepted_in_one_session() {
        let mut session = session_with_containers(&[]);
        session.clusters.insert(
            named_key("alpha"),
            ClusterServices::new(vec![cluster_node("alpha-node-0")]),
        );

        assert!(ensure_session_accepts_cluster(Some(&session), &named_key("beta")).is_ok());
    }

    #[test]
    fn duplicate_named_cluster_is_rejected() {
        let mut session = session_with_containers(&[]);
        session.clusters.insert(
            named_key("alpha"),
            ClusterServices::new(vec![cluster_node("alpha-node-0")]),
        );

        let error = ensure_session_accepts_cluster(Some(&session), &named_key("alpha"))
            .err()
            .expect("duplicate cluster names must be rejected");

        assert!(matches!(
            error,
            ComposeRunnerError::ClusterAlreadyProvisioned { name } if name == "alpha"
        ));
    }

    #[tokio::test]
    async fn a_second_unnamed_cluster_is_rejected_with_naming_guidance() {
        let mut session = session_with_containers(&[]);
        session.clusters.insert(
            ClusterKey::Unnamed,
            ClusterServices::new(vec![cluster_node("node-0")]),
        );
        let provisioner = ComposeProvisioner::default();
        provisioner.store_session(session);

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
        let mut session = session_with_containers(&[]);
        session.clusters.insert(
            named_key("alpha"),
            ClusterServices::new(vec![cluster_node("alpha-node-0")]),
        );
        let provisioner = ComposeProvisioner::default();
        provisioner.store_session(session);

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

    #[tokio::test]
    async fn a_poisoned_session_fails_fast_without_burning_retries() {
        let mut session = session_with_containers(&[]);
        session.poisoned = true;
        let provisioner = ComposeProvisioner::default();
        provisioner.store_session(session);

        let policy = DeploymentPolicy {
            retry_policy: Some(RetryPolicy::new(
                3,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )),
            cleanup_policy: CleanupPolicy::new(false),
            ..DeploymentPolicy::default()
        };
        let request = ClusterRequest::<TestEnv>::managed(TestTopology).with_policy(policy);

        let error = tokio::time::timeout(
            Duration::from_secs(5),
            provisioner.provision_cluster(request),
        )
        .await
        .expect("terminal errors must not wait out retry backoff")
        .err()
        .expect("poisoned session must be rejected");

        assert!(matches!(
            error.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::SessionPoisoned)
        ));
    }

    #[test]
    fn named_owner_and_unnamed_joiner_use_distinct_cfgsync_paths() {
        let session = session_with_containers(&[]);
        let named = session_cfgsync_path(&session.project, &named_key("alpha"));
        let unnamed = session_cfgsync_path(&session.project, &ClusterKey::Unnamed);

        assert_ne!(
            named, unnamed,
            "a named owner and an unnamed joiner must never share a cfgsync file"
        );
        assert!(named.ends_with("stack/cfgsync-alpha.yaml"));
        assert!(unnamed.ends_with("stack/cfgsync.yaml"));
        assert_eq!(
            named_key("alpha").cfgsync_file_name(),
            "cfgsync-alpha.yaml",
            "owners derive their workspace cfgsync file from the same key mapping"
        );
    }

    #[test]
    fn stale_poison_request_leaves_a_newer_session_untouched() {
        let provisioner = ComposeProvisioner::default();
        let mut session = session_with_containers(&[]);
        session.generation = 2;
        provisioner.store_session(session);

        poison_session(&provisioner, 1);

        let live = provisioner
            .session_snapshot()
            .expect("live session must survive a stale poison request");
        assert!(!live.poisoned, "stale poison requests must be ignored");

        poison_session(&provisioner, 2);
        assert!(
            provisioner
                .session_snapshot()
                .expect("session must stay stored")
                .poisoned,
            "matching poison requests must still poison the session"
        );
    }

    #[test]
    fn terminal_errors_are_not_retryable() {
        assert!(ComposeRunnerError::SessionPoisoned.is_terminal());
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
        assert!(ComposeRunnerError::OnDemandUnsupported.is_terminal());
        assert!(
            ComposeRunnerError::ExtensionRollback {
                failure: Box::new(ComposeRunnerError::RuntimePreflight),
                rollback: Box::new(ComposeRunnerError::RuntimePreflight),
            }
            .is_terminal()
        );
        assert!(!ComposeRunnerError::DockerUnavailable.is_terminal());
        assert!(!ComposeRunnerError::RuntimePreflight.is_terminal());
    }
}
