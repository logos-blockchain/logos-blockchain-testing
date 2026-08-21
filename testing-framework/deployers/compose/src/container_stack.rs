use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context as _, anyhow};
use async_trait::async_trait;
use futures::future::try_join_all;
use testing_framework_container::{
    ContainerEndpoint, ContainerReadiness, ContainerRestartPolicy, ContainerServiceControl,
    ContainerServiceHandle, ContainerServiceSpec, ContainerStackHandle, ContainerStackProvisioner,
    ContainerStackRequest, ProvisionedContainerStack,
};
use testing_framework_core::{
    adjust_timeout,
    scenario::{CleanupGuard, DynError},
};
use tokio::{net::TcpStream, sync::Mutex as AsyncMutex, time::Instant};
use tracing::warn;

use crate::{
    descriptor::{ComposeDescriptor, EnvEntry, NodeDescriptor},
    docker::{ensure_docker_available, ensure_image_present, workspace::ComposeWorkspace},
    infrastructure::{
        ports::compose_runner_host, project::ComposeProject, template::write_compose_file,
    },
    lifecycle::cleanup::RunnerCleanup,
};

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(200);
type RunnerPorts = BTreeMap<String, BTreeMap<String, u16>>;
const PORT_BIND_ATTEMPTS: usize = 3;

struct RunnerPortReservation {
    ports: RunnerPorts,
    listeners: Vec<StdTcpListener>,
}

impl RunnerPortReservation {
    fn ports(&self) -> &RunnerPorts {
        &self.ports
    }

    fn release(self) -> RunnerPorts {
        let Self { ports, listeners } = self;
        drop(listeners);
        ports
    }
}

/// Provisions portable container stacks through Docker Compose.
#[derive(Clone, Default)]
pub struct ComposeContainerProvisioner {
    inner: Arc<ComposeProvisionerInner>,
}

#[derive(Default)]
struct ComposeProvisionerInner {
    mutation: AsyncMutex<()>,
    session: Mutex<Option<ComposeSession>>,
}

#[derive(Clone)]
struct ComposeSession {
    project: ComposeProject,
    services: Vec<ContainerServiceSpec>,
    runner_ports: RunnerPorts,
    poisoned: bool,
}

struct ComposeSessionCleanup {
    inner: Arc<ComposeProvisionerInner>,
    cleanup: Option<RunnerCleanup>,
}

struct ComposeContainerControl {
    operation: AsyncMutex<()>,
    project: ComposeProject,
    service: ContainerServiceSpec,
    endpoints: BTreeMap<String, ContainerEndpoint>,
}

impl ComposeContainerControl {
    async fn wait_for_readiness(&self) -> Result<(), DynError> {
        if let Err(source) = wait_for_service_readiness(&self.service, &self.endpoints).await {
            self.project.dump_logs().await;
            return Err(source);
        }
        Ok(())
    }
}

#[async_trait]
impl ContainerServiceControl for ComposeContainerControl {
    async fn start(&self) -> Result<(), DynError> {
        let _operation = self.operation.lock().await;
        self.project.start_service(self.service.name()).await?;
        self.wait_for_readiness().await
    }

    async fn stop(&self) -> Result<(), DynError> {
        let _operation = self.operation.lock().await;
        self.project.stop_service(self.service.name()).await?;
        Ok(())
    }

    async fn restart(&self) -> Result<(), DynError> {
        let _operation = self.operation.lock().await;
        self.project.restart_service(self.service.name()).await?;
        self.wait_for_readiness().await
    }

    async fn wait_ready(&self) -> Result<(), DynError> {
        let _operation = self.operation.lock().await;
        if !self.project.service_is_running(self.service.name()).await? {
            return Err(anyhow!("service '{}' is not running", self.service.name()).into());
        }
        self.wait_for_readiness().await
    }

    async fn is_running(&self) -> Result<bool, DynError> {
        let _operation = self.operation.lock().await;
        self.project.service_is_running(self.service.name()).await
    }
}

impl CleanupGuard for ComposeSessionCleanup {
    fn cleanup(mut self: Box<Self>) {
        *self
            .inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        if let Some(cleanup) = self.cleanup.take() {
            Box::new(cleanup).cleanup();
        }
    }
}

#[async_trait]
impl ContainerStackProvisioner for ComposeContainerProvisioner {
    async fn provision_container_stack(
        &self,
        request: ContainerStackRequest,
    ) -> Result<ProvisionedContainerStack, DynError> {
        validate_request(&request)?;
        ensure_docker_available().await?;
        ensure_service_images_present(request.services()).await?;

        let _mutation = self.inner.mutation.lock().await;

        let session = self
            .inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match session {
            Some(session) if session.poisoned => Err(anyhow!(
                "compose service session is unavailable after a failed extension"
            )
            .into()),
            Some(session) => self.extend_session(session, request).await,
            None => self.start_session(request).await,
        }
    }
}

impl ComposeContainerProvisioner {
    async fn start_session(
        &self,
        request: ContainerStackRequest,
    ) -> Result<ProvisionedContainerStack, DynError> {
        let workspace = ComposeWorkspace::create()?;
        let root = workspace.root_path().to_path_buf();
        write_service_files(&root, request.services())?;

        let project = ComposeProject::for_workspace(&workspace, request.name());
        let cleanup = RunnerCleanup::new(project.clone(), workspace, None);

        let runner_ports = {
            let mut attempt = 0;
            loop {
                attempt += 1;
                let reservation = match reserve_runner_ports(request.services()) {
                    Ok(reservation) => reservation,
                    Err(source) => {
                        cleanup_failed_stack(cleanup, &project).await;
                        return Err(source);
                    }
                };
                let descriptor = match compose_descriptor(request.services(), reservation.ports()) {
                    Ok(descriptor) => descriptor,
                    Err(source) => {
                        cleanup_failed_stack(cleanup, &project).await;
                        return Err(source);
                    }
                };

                let project_access = project.lock_mutation().await;
                if let Err(source) = write_compose_file(&descriptor, project.compose_file()) {
                    drop(project_access);
                    cleanup_failed_stack(cleanup, &project).await;
                    return Err(source.into());
                }
                let runner_ports = reservation.release();
                let result = project.up_unlocked().await;

                match result {
                    Ok(()) => {
                        drop(project_access);
                        break runner_ports;
                    }
                    Err(source) if source.is_port_conflict() && attempt < PORT_BIND_ATTEMPTS => {
                        let cleanup_result = project.down_unlocked().await;
                        drop(project_access);
                        if let Err(cleanup_error) = cleanup_result {
                            cleanup_failed_stack(cleanup, &project).await;
                            return Err(anyhow!(
                                "compose port conflict retry cleanup failed: {cleanup_error}"
                            )
                            .into());
                        }
                    }
                    Err(source) => {
                        drop(project_access);
                        cleanup_failed_stack(cleanup, &project).await;
                        return Err(source.into());
                    }
                }
            }
        };

        let handles = match build_handles(request.services(), &project).await {
            Ok(handles) => handles,
            Err(source) => {
                cleanup_failed_stack(cleanup, &project).await;
                return Err(source);
            }
        };

        *self
            .inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ComposeSession {
            project,
            services: request.services().to_vec(),
            runner_ports,
            poisoned: false,
        });

        Ok(ProvisionedContainerStack::new(
            ContainerStackHandle::new(handles),
            Some(Box::new(ComposeSessionCleanup {
                inner: Arc::clone(&self.inner),
                cleanup: Some(cleanup),
            })),
        ))
    }

    async fn extend_session(
        &self,
        session: ComposeSession,
        request: ContainerStackRequest,
    ) -> Result<ProvisionedContainerStack, DynError> {
        ensure_names_available(&session.services, request.services())?;

        let mut services = session.services.clone();
        services.extend_from_slice(request.services());
        let service_names = request
            .services()
            .iter()
            .map(|service| service.name().to_owned())
            .collect::<Vec<_>>();

        for attempt in 1..=PORT_BIND_ATTEMPTS {
            write_service_files(session.project.root(), request.services())?;
            let reservation = reserve_runner_ports(request.services())?;
            let mut runner_ports = session.runner_ports.clone();
            runner_ports.extend(reservation.ports().clone());
            let descriptor = compose_descriptor(&services, &runner_ports)?;

            let project_access = session.project.lock_mutation().await;
            if let Err(source) = write_compose_file(&descriptor, session.project.compose_file()) {
                let restore = write_compose_file(
                    &compose_descriptor(&session.services, &session.runner_ports)?,
                    session.project.compose_file(),
                );
                drop(project_access);
                if let Err(restore_error) = restore {
                    self.poison_session(&session, &services, &runner_ports);
                    return Err(anyhow!(
                        "writing extended compose file failed: {source}; restoring it failed: {restore_error}"
                    )
                    .into());
                }
                remove_service_files(session.project.root(), &service_names);
                return Err(source.into());
            }
            drop(reservation.release());
            let mut start_error = None;
            for name in &service_names {
                if let Err(source) = session.project.up_service_unlocked(name).await {
                    start_error = Some(source);
                    break;
                }
            }
            drop(project_access);

            if let Some(source) = start_error {
                session.project.dump_logs().await;
                let retry = source.is_port_conflict() && attempt < PORT_BIND_ATTEMPTS;
                let failure: DynError = source.into();
                if let Err(rollback_error) = self
                    .rollback_or_poison(&session, &services, &runner_ports, &service_names, retry)
                    .await
                {
                    return Err(anyhow!(
                        "compose extension start failed: {failure}; rollback also failed: \
                         {rollback_error}"
                    )
                    .into());
                }
                if retry {
                    continue;
                }
                return Err(failure);
            }

            let handles = match build_handles(request.services(), &session.project).await {
                Ok(handles) => handles,
                Err(source) => {
                    session.project.dump_logs().await;
                    if let Err(rollback_error) = self
                        .rollback_or_poison(
                            &session,
                            &services,
                            &runner_ports,
                            &service_names,
                            false,
                        )
                        .await
                    {
                        return Err(anyhow!(
                            "compose extension readiness failed: {source}; rollback also failed: \
                             {rollback_error}"
                        )
                        .into());
                    }
                    return Err(source);
                }
            };

            *self
                .inner
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ComposeSession {
                services,
                runner_ports,
                poisoned: false,
                ..session
            });

            return Ok(ProvisionedContainerStack::new(
                ContainerStackHandle::new(handles),
                None,
            ));
        }

        Err("compose port allocation retries exhausted".into())
    }

    async fn rollback_or_poison(
        &self,
        previous: &ComposeSession,
        services: &[ContainerServiceSpec],
        runner_ports: &RunnerPorts,
        added_services: &[String],
        retrying: bool,
    ) -> Result<(), DynError> {
        if preserve_requested() && !retrying {
            self.poison_session(previous, services, runner_ports);
            return Ok(());
        }

        let rollback = async {
            let _project_access = previous.project.lock_mutation().await;
            previous
                .project
                .remove_services_unlocked(added_services)
                .await?;
            write_compose_file(
                &compose_descriptor(&previous.services, &previous.runner_ports)?,
                previous.project.compose_file(),
            )?;
            Ok::<(), DynError>(())
        }
        .await;

        if let Err(source) = rollback {
            self.poison_session(previous, services, runner_ports);
            return Err(anyhow!("compose extension rollback failed: {source}").into());
        }

        remove_service_files(previous.project.root(), added_services);
        Ok(())
    }

    fn poison_session(
        &self,
        previous: &ComposeSession,
        services: &[ContainerServiceSpec],
        runner_ports: &RunnerPorts,
    ) {
        *self
            .inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ComposeSession {
            services: services.to_vec(),
            runner_ports: runner_ports.clone(),
            poisoned: true,
            ..previous.clone()
        });
    }
}

async fn ensure_service_images_present(services: &[ContainerServiceSpec]) -> Result<(), DynError> {
    for service in services {
        ensure_image_present(service.image(), None).await?;
    }
    Ok(())
}

fn ensure_names_available(
    existing: &[ContainerServiceSpec],
    requested: &[ContainerServiceSpec],
) -> Result<(), DynError> {
    let existing = existing
        .iter()
        .map(ContainerServiceSpec::name)
        .collect::<BTreeSet<_>>();
    if let Some(name) = requested
        .iter()
        .map(ContainerServiceSpec::name)
        .find(|name| existing.contains(name))
    {
        return Err(anyhow!("service '{name}' is already provisioned").into());
    }
    Ok(())
}

fn preserve_requested() -> bool {
    env::var_os("COMPOSE_RUNNER_PRESERVE").is_some()
        || env::var_os("TESTNET_RUNNER_PRESERVE").is_some()
}

fn remove_service_files(root: &Path, services: &[String]) {
    for service in services {
        let directory = root.join("stack/services").join(service);
        if let Err(source) = fs::remove_dir_all(&directory)
            && source.kind() != std::io::ErrorKind::NotFound
        {
            warn!(?source, path = %directory.display(), "failed to remove rolled-back service files");
        }
    }
}

async fn cleanup_failed_stack(cleanup: RunnerCleanup, project: &ComposeProject) {
    project.dump_logs().await;
    Box::new(cleanup).cleanup();
}

fn validate_request(request: &ContainerStackRequest) -> Result<(), DynError> {
    if request.services().is_empty() {
        return Err(anyhow!("service stack '{}' contains no services", request.name()).into());
    }

    let mut names = BTreeSet::new();
    for service in request.services() {
        validate_service(service)?;
        if !names.insert(service.name()) {
            return Err(anyhow!("duplicate service name '{}'", service.name()).into());
        }
    }
    Ok(())
}

fn validate_service(service: &ContainerServiceSpec) -> Result<(), DynError> {
    if !is_valid_service_name(service.name()) {
        return Err(anyhow!(
            "invalid service name '{}'; use a Kubernetes-compatible DNS label",
            service.name()
        )
        .into());
    }
    if service.image().trim().is_empty() {
        return Err(anyhow!("service '{}' has no image", service.name()).into());
    }
    if service.command().is_empty() {
        return Err(anyhow!("service '{}' has no command", service.name()).into());
    }
    if let Some(key) = service
        .environment()
        .keys()
        .find(|key| !is_valid_env_key(key))
    {
        return Err(anyhow!(
            "service '{}' contains invalid environment key '{}'",
            service.name(),
            key
        )
        .into());
    }

    let mut ports = BTreeSet::new();
    for port in service.ports() {
        if !is_valid_port_name(port.name()) || !ports.insert(port.name()) {
            return Err(anyhow!(
                "service '{}' contains an invalid or duplicate port name '{}'",
                service.name(),
                port.name()
            )
            .into());
        }
        if port.container_port() == 0 {
            return Err(anyhow!(
                "service '{}' port '{}' uses invalid container port 0",
                service.name(),
                port.name()
            )
            .into());
        }
    }

    for file in service.files() {
        let name = Path::new(file.name());
        if name.file_name().and_then(|value| value.to_str()) != Some(file.name()) {
            return Err(anyhow!(
                "service '{}' file '{}' must be a plain file name",
                service.name(),
                file.name()
            )
            .into());
        }
        if !file.mount_path().is_absolute() {
            return Err(anyhow!(
                "service '{}' mount path '{}' must be absolute",
                service.name(),
                file.mount_path().display()
            )
            .into());
        }
    }

    if let Some(readiness) = service.readiness() {
        let port_name = readiness_port(readiness);
        let Some(port) = service.ports().iter().find(|port| port.name() == port_name) else {
            return Err(anyhow!(
                "service '{}' readiness references unknown port '{}'",
                service.name(),
                port_name
            )
            .into());
        };
        if !port.is_published() {
            return Err(anyhow!(
                "service '{}' readiness port '{}' must be published for Compose",
                service.name(),
                port_name
            )
            .into());
        }
    }
    Ok(())
}

fn is_valid_service_name(name: &str) -> bool {
    is_valid_dns_label(name, 63)
}

fn is_valid_port_name(name: &str) -> bool {
    is_valid_dns_label(name, 15)
}

fn is_valid_env_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_valid_dns_label(name: &str, max_len: usize) -> bool {
    !name.is_empty()
        && name.len() <= max_len
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn readiness_port(readiness: &ContainerReadiness) -> &str {
    match readiness {
        ContainerReadiness::Http { port, .. } | ContainerReadiness::Tcp { port, .. } => port,
    }
}

fn needs_runner_port(service: &ContainerServiceSpec, port_name: &str) -> bool {
    service
        .ports()
        .iter()
        .any(|port| port.name() == port_name && port.is_published())
}

fn write_service_files(root: &Path, services: &[ContainerServiceSpec]) -> Result<(), DynError> {
    for service in services {
        let directory = root.join("stack/services").join(service.name());
        fs::create_dir_all(&directory)
            .with_context(|| format!("creating service directory {}", directory.display()))?;
        for file in service.files() {
            let path = directory.join(file.name());
            fs::write(&path, file.contents())
                .with_context(|| format!("writing service file {}", path.display()))?;
        }
    }
    Ok(())
}

fn reserve_runner_ports(
    services: &[ContainerServiceSpec],
) -> Result<RunnerPortReservation, DynError> {
    let mut listeners = Vec::new();
    let mut mappings = RunnerPorts::new();
    for service in services {
        let mut service_ports = BTreeMap::new();
        for port in service
            .ports()
            .iter()
            .filter(|port| needs_runner_port(service, port.name()))
        {
            let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).with_context(|| {
                format!(
                    "allocating host port for service '{}' port '{}'",
                    service.name(),
                    port.name()
                )
            })?;
            let host_port = listener.local_addr()?.port();
            listeners.push(listener);
            service_ports.insert(port.name().to_owned(), host_port);
        }
        mappings.insert(service.name().to_owned(), service_ports);
    }
    Ok(RunnerPortReservation {
        ports: mappings,
        listeners,
    })
}

fn compose_descriptor(
    services: &[ContainerServiceSpec],
    runner_ports: &RunnerPorts,
) -> Result<ComposeDescriptor, DynError> {
    Ok(ComposeDescriptor::new(
        services
            .iter()
            .map(|service| node_descriptor(service, runner_ports))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn node_descriptor(
    service: &ContainerServiceSpec,
    runner_ports: &RunnerPorts,
) -> Result<NodeDescriptor, DynError> {
    let volumes = service
        .files()
        .iter()
        .map(|file| {
            format!(
                "./stack/services/{}/{}:{}:ro",
                service.name(),
                file.name(),
                file.mount_path().display()
            )
        })
        .collect();
    let published_ports = service
        .ports()
        .iter()
        .filter(|port| needs_runner_port(service, port.name()))
        .map(|port| {
            let host_port = runner_ports
                .get(service.name())
                .and_then(|ports| ports.get(port.name()))
                .copied()
                .ok_or_else(|| {
                    anyhow!(
                        "host port is missing for service '{}' port '{}'",
                        service.name(),
                        port.name()
                    )
                })?;
            Ok(format!("127.0.0.1:{host_port}:{}", port.container_port()))
        })
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    let container_ports = service
        .ports()
        .iter()
        .map(|port| port.container_port())
        .collect();
    let environment = service
        .environment()
        .iter()
        .map(|(key, value)| EnvEntry::literal(key, value))
        .collect();

    Ok(NodeDescriptor::new(
        service.name(),
        service.image(),
        service.command().to_vec(),
        volumes,
        Vec::new(),
        published_ports,
        container_ports,
        environment,
        None,
    )
    .without_debug_capabilities()
    .with_restart_policy(match service.restart_policy() {
        ContainerRestartPolicy::Never => "no",
        ContainerRestartPolicy::OnFailure => "on-failure",
        ContainerRestartPolicy::Always => "always",
    }))
}

async fn build_handles(
    services: &[ContainerServiceSpec],
    project: &ComposeProject,
) -> Result<BTreeMap<String, ContainerServiceHandle>, DynError> {
    let host = compose_runner_host();
    let handles = try_join_all(
        services
            .iter()
            .map(|service| build_handle(service, project, &host)),
    )
    .await?;

    Ok(handles.into_iter().collect())
}

async fn build_handle(
    service: &ContainerServiceSpec,
    project: &ComposeProject,
    host: &str,
) -> Result<(String, ContainerServiceHandle), DynError> {
    let mut endpoints = BTreeMap::new();
    let mut readiness_endpoints = BTreeMap::new();
    let internal_endpoints = service
        .ports()
        .iter()
        .map(|port| {
            (
                port.name().to_owned(),
                ContainerEndpoint::new(service.name(), port.container_port()),
            )
        })
        .collect();
    for port in service
        .ports()
        .iter()
        .filter(|port| needs_runner_port(service, port.name()))
    {
        let host_port = project
            .resolve_service_port(service.name(), port.container_port())
            .await?;
        let endpoint = ContainerEndpoint::new(host, host_port);
        readiness_endpoints.insert(port.name().to_owned(), endpoint.clone());
        if port.is_published() {
            endpoints.insert(port.name().to_owned(), endpoint);
        }
    }

    wait_for_service_readiness(service, &readiness_endpoints).await?;
    let control = Arc::new(ComposeContainerControl {
        operation: AsyncMutex::new(()),
        project: project.clone(),
        service: service.clone(),
        endpoints: readiness_endpoints,
    });
    Ok((
        service.name().to_owned(),
        ContainerServiceHandle::new(service.name(), endpoints, internal_endpoints, control),
    ))
}

async fn wait_for_service_readiness(
    service: &ContainerServiceSpec,
    endpoints: &BTreeMap<String, ContainerEndpoint>,
) -> Result<(), DynError> {
    let Some(readiness) = service.readiness() else {
        return Ok(());
    };
    let endpoint = endpoints
        .get(readiness_port(readiness))
        .ok_or_else(|| anyhow!("service '{}' readiness endpoint is missing", service.name()))?;

    match readiness {
        ContainerReadiness::Http { path, timeout, .. } => {
            wait_for_http(endpoint, path, *timeout).await
        }
        ContainerReadiness::Tcp { timeout, .. } => wait_for_tcp(endpoint, *timeout).await,
    }
}

async fn wait_for_http(
    endpoint: &ContainerEndpoint,
    path: &str,
    timeout: Duration,
) -> Result<(), DynError> {
    let url = format!(
        "http://{}{}{}",
        endpoint.authority(),
        if path.starts_with('/') { "" } else { "/" },
        path
    );
    let deadline = Instant::now() + adjust_timeout(timeout);
    let client = reqwest::Client::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!("service readiness timed out for {url}").into());
        }
        if tokio::time::timeout(remaining, client.get(&url).send())
            .await
            .is_ok_and(|result| result.is_ok_and(|response| response.status().is_success()))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow!("service readiness timed out for {url}").into());
        }
        tokio::time::sleep(READINESS_POLL_INTERVAL).await;
    }
}

async fn wait_for_tcp(endpoint: &ContainerEndpoint, timeout: Duration) -> Result<(), DynError> {
    let deadline = Instant::now() + adjust_timeout(timeout);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!(
                "service TCP readiness timed out for {}",
                endpoint.authority()
            )
            .into());
        }
        if tokio::time::timeout(
            remaining,
            TcpStream::connect((endpoint.host(), endpoint.port())),
        )
        .await
        .is_ok_and(|result| result.is_ok())
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "service TCP readiness timed out for {}",
                endpoint.authority()
            )
            .into());
        }
        tokio::time::sleep(READINESS_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use testing_framework_container::{
        ContainerFile, ContainerPort, ContainerReadiness, ContainerServiceSpec,
        ContainerStackRequest,
    };

    use super::{
        compose_descriptor, ensure_names_available, remove_service_files, reserve_runner_ports,
        validate_request, write_service_files,
    };
    use crate::infrastructure::template::write_compose_file;

    fn service(name: &str) -> ContainerServiceSpec {
        ContainerServiceSpec::new(name, "example:local")
            .with_command(["/bin/example"])
            .with_port(ContainerPort::new("api", 8080).published())
            .with_readiness(ContainerReadiness::http("api", "/ready"))
    }

    #[test]
    fn request_rejects_duplicate_service_names() {
        let request = ContainerStackRequest::new("duplicate")
            .with_service(service("worker"))
            .with_service(service("worker"));

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn extension_rejects_a_name_already_in_the_session() {
        assert!(ensure_names_available(&[service("worker")], &[service("worker")]).is_err());
    }

    #[test]
    fn published_ports_are_explicit_and_distinct() {
        let services = [service("queue"), service("worker")];
        let reservation = reserve_runner_ports(&services).unwrap();
        let ports = reservation.ports();
        let queue = ports["queue"]["api"];
        let worker = ports["worker"]["api"];

        assert_ne!(queue, worker);
        let descriptor = compose_descriptor(&services, ports).unwrap();
        assert_eq!(descriptor.nodes().len(), 2);
        assert!(descriptor.nodes()[0].ports()[0].contains(&queue.to_string()));
        assert!(descriptor.nodes()[1].ports()[0].contains(&worker.to_string()));
    }

    #[test]
    fn environment_values_render_as_literal_compose_data() {
        let services = [service("worker").with_env("MESSAGE", "say \"hi\"\n$HOME")];
        let reservation = reserve_runner_ports(&services).unwrap();
        let descriptor = compose_descriptor(&services, reservation.ports()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("compose.yml");

        write_compose_file(&descriptor, &output).unwrap();
        let rendered = fs::read_to_string(output).unwrap();

        assert!(rendered.contains("\"MESSAGE\": \"say \\\"hi\\\"\\n$$HOME\""));
        assert!(rendered.contains("restart: \"no\""));
    }

    #[test]
    fn environment_keys_must_be_portable_identifiers() {
        let request = ContainerStackRequest::new("invalid-env")
            .with_service(service("worker").with_env("BAD-KEY", "value"));

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn retry_preparation_recreates_removed_service_files() {
        let directory = tempfile::tempdir().unwrap();
        let services = [service("worker").with_file(ContainerFile::new(
            "config.yaml",
            "/etc/worker/config.yaml",
            b"attempt: retry".to_vec(),
        ))];
        let path = directory.path().join("stack/services/worker/config.yaml");

        write_service_files(directory.path(), &services).unwrap();
        remove_service_files(directory.path(), &["worker".to_owned()]);
        assert!(!path.exists());

        write_service_files(directory.path(), &services).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"attempt: retry");
    }
}
