//! Portable contracts for containerized test workloads.
//!
//! Application integrations describe container intent with
//! [`ContainerServiceSpec`]. Compose and Kubernetes provisioners translate
//! that intent into backend resources and return portable lifecycle handles.
//! This crate contains no Docker, Helm, Kubernetes, Local-process, or
//! application-composition implementation.

#![warn(missing_docs)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use testing_framework_core::scenario::{CleanupGuard, DynError};

/// A named port exposed by one managed container.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerPort {
    name: String,
    container_port: u16,
    publish: bool,
}

impl ContainerPort {
    /// Describes a port that is initially only reachable by sibling services.
    #[must_use]
    pub fn new(name: impl Into<String>, container_port: u16) -> Self {
        Self {
            name: name.into(),
            container_port,
            publish: false,
        }
    }

    /// Makes the port reachable from the test runner as well.
    #[must_use]
    pub const fn published(mut self) -> Self {
        self.publish = true;
        self
    }

    /// Returns the stable name used to look the port up from a service handle.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the port listened on inside the service runtime.
    #[must_use]
    pub const fn container_port(&self) -> u16 {
        self.container_port
    }

    /// Returns whether the test runner needs an externally reachable mapping.
    #[must_use]
    pub const fn is_published(&self) -> bool {
        self.publish
    }
}

/// A generated file mounted into one managed container.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerFile {
    name: String,
    mount_path: PathBuf,
    contents: Vec<u8>,
}

impl ContainerFile {
    /// Describes a generated file and its destination inside the service.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        mount_path: impl Into<PathBuf>,
        contents: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            name: name.into(),
            mount_path: mount_path.into(),
            contents: contents.into(),
        }
    }

    /// Returns the backend workspace file name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the absolute destination path inside the service.
    #[must_use]
    pub fn mount_path(&self) -> &Path {
        &self.mount_path
    }

    /// Returns the generated file contents.
    #[must_use]
    pub fn contents(&self) -> &[u8] {
        &self.contents
    }
}

/// Readiness condition evaluated after a service is started.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContainerReadiness {
    /// Poll an HTTP endpoint on a named runner-accessible port.
    Http {
        /// Port name from [`ContainerServiceSpec::with_port`].
        port: String,
        /// Absolute HTTP path, for example `/health/ready`.
        path: String,
        /// Maximum time to wait for readiness.
        timeout: Duration,
    },
    /// Wait for a named runner-accessible port to accept TCP connections.
    Tcp {
        /// Port name from [`ContainerServiceSpec::with_port`].
        port: String,
        /// Maximum time to wait for readiness.
        timeout: Duration,
    },
}

impl ContainerReadiness {
    /// Creates an HTTP readiness condition.
    #[must_use]
    pub fn http(port: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Http {
            port: port.into(),
            path: path.into(),
            timeout: Duration::from_secs(60),
        }
    }

    /// Creates a TCP readiness condition.
    #[must_use]
    pub fn tcp(port: impl Into<String>) -> Self {
        Self::Tcp {
            port: port.into(),
            timeout: Duration::from_secs(60),
        }
    }

    /// Overrides the maximum readiness wait.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        match &mut self {
            Self::Http {
                timeout: current, ..
            }
            | Self::Tcp {
                timeout: current, ..
            } => *current = timeout,
        }
        self
    }
}

/// Behavior after the container process exits unexpectedly.
///
/// The default is [`Self::Never`] so a crash remains visible to tests instead
/// of being hidden by backend-specific automatic recovery.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ContainerRestartPolicy {
    /// Leave the service stopped after its process exits.
    #[default]
    Never,
    /// Restart the service only after a non-zero exit.
    OnFailure,
    /// Always restart the service after it exits.
    Always,
}

/// Portable description of one containerized application service.
///
/// Compose and Kubernetes can realize the same service contract. Application
/// repositories remain responsible for images, commands, configuration, and
/// readiness semantics.
#[derive(Clone, Debug)]
pub struct ContainerServiceSpec {
    name: String,
    image: String,
    command: Vec<String>,
    environment: BTreeMap<String, String>,
    ports: Vec<ContainerPort>,
    files: Vec<ContainerFile>,
    readiness: Option<ContainerReadiness>,
    restart_policy: ContainerRestartPolicy,
}

impl ContainerServiceSpec {
    /// Starts a service description with its stable name and container image.
    #[must_use]
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            command: Vec::new(),
            environment: BTreeMap::new(),
            ports: Vec::new(),
            files: Vec::new(),
            readiness: None,
            restart_policy: ContainerRestartPolicy::Never,
        }
    }

    /// Sets the complete executable and argument vector.
    #[must_use]
    pub fn with_command<I, S>(mut self, command: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.command = command.into_iter().map(Into::into).collect();
        self
    }

    /// Adds one environment variable.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    /// Adds one service port.
    #[must_use]
    pub fn with_port(mut self, port: ContainerPort) -> Self {
        self.ports.push(port);
        self
    }

    /// Adds one generated configuration file.
    #[must_use]
    pub fn with_file(mut self, file: ContainerFile) -> Self {
        self.files.push(file);
        self
    }

    /// Adds a readiness condition.
    #[must_use]
    pub fn with_readiness(mut self, readiness: ContainerReadiness) -> Self {
        self.readiness = Some(readiness);
        self
    }

    /// Sets the behavior after the container process exits unexpectedly.
    #[must_use]
    pub const fn with_restart_policy(mut self, policy: ContainerRestartPolicy) -> Self {
        self.restart_policy = policy;
        self
    }

    /// Returns the stable service name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the container image reference.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the complete executable and argument vector.
    #[must_use]
    pub fn command(&self) -> &[String] {
        &self.command
    }

    /// Returns configured environment variables.
    #[must_use]
    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    /// Returns declared service ports.
    #[must_use]
    pub fn ports(&self) -> &[ContainerPort] {
        &self.ports
    }

    /// Returns generated configuration files.
    #[must_use]
    pub fn files(&self) -> &[ContainerFile] {
        &self.files
    }

    /// Returns the readiness condition, when one was configured.
    #[must_use]
    pub const fn readiness(&self) -> Option<&ContainerReadiness> {
        self.readiness.as_ref()
    }

    /// Returns the requested process restart behavior.
    #[must_use]
    pub const fn restart_policy(&self) -> ContainerRestartPolicy {
        self.restart_policy
    }
}

/// One group of services added to a backend deployment session.
///
/// A composed app may submit several requests in dependency order. All calls
/// made through clones of the same provisioner join the same backend network
/// or namespace until scenario cleanup.
#[derive(Clone, Debug)]
pub struct ContainerStackRequest {
    name: String,
    services: Vec<ContainerServiceSpec>,
}

impl ContainerStackRequest {
    /// Creates an empty named service stack.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            services: Vec::new(),
        }
    }

    /// Adds one service to the stack.
    #[must_use]
    pub fn with_service(mut self, service: ContainerServiceSpec) -> Self {
        self.services.push(service);
        self
    }

    /// Returns the human-readable stack name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns all services in declaration order.
    #[must_use]
    pub fn services(&self) -> &[ContainerServiceSpec] {
        &self.services
    }
}

/// Backend-resolved endpoint for one named service port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerEndpoint {
    host: String,
    port: u16,
}

impl ContainerEndpoint {
    /// Creates a resolved endpoint.
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// Returns the resolved host.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the resolved port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns `host:port` for URL or socket construction.
    #[must_use]
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Runtime access to one managed service.
#[derive(Clone)]
pub struct ContainerServiceHandle {
    name: String,
    endpoints: BTreeMap<String, ContainerEndpoint>,
    internal_endpoints: BTreeMap<String, ContainerEndpoint>,
    control: Arc<dyn ContainerServiceControl>,
}

impl ContainerServiceHandle {
    /// Creates a service handle from runner and backend-internal endpoints.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        endpoints: BTreeMap<String, ContainerEndpoint>,
        internal_endpoints: BTreeMap<String, ContainerEndpoint>,
        control: Arc<dyn ContainerServiceControl>,
    ) -> Self {
        Self {
            name: name.into(),
            endpoints,
            internal_endpoints,
            control,
        }
    }

    /// Returns the stable service name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns a published endpoint by port name.
    #[must_use]
    pub fn endpoint(&self, port: &str) -> Option<&ContainerEndpoint> {
        self.endpoints.get(port)
    }

    /// Returns the address used by sibling services inside the backend.
    #[must_use]
    pub fn internal_endpoint(&self, port: &str) -> Option<&ContainerEndpoint> {
        self.internal_endpoints.get(port)
    }

    /// Starts this service and waits for its declared readiness condition.
    pub async fn start(&self) -> Result<(), DynError> {
        self.control.start().await
    }

    /// Stops this service while leaving the rest of the stack running.
    pub async fn stop(&self) -> Result<(), DynError> {
        self.control.stop().await
    }

    /// Restarts this service and waits for its declared readiness condition.
    pub async fn restart(&self) -> Result<(), DynError> {
        self.control.restart().await
    }

    /// Waits for this service's declared readiness condition.
    pub async fn wait_ready(&self) -> Result<(), DynError> {
        self.control.wait_ready().await
    }

    /// Returns whether the backend currently reports this service as running.
    pub async fn is_running(&self) -> Result<bool, DynError> {
        self.control.is_running().await
    }
}

/// Backend-specific lifecycle operations hidden behind a portable service
/// handle.
#[async_trait]
pub trait ContainerServiceControl: Send + Sync + 'static {
    /// Starts one service.
    async fn start(&self) -> Result<(), DynError>;

    /// Stops one service.
    async fn stop(&self) -> Result<(), DynError>;

    /// Restarts one service.
    async fn restart(&self) -> Result<(), DynError>;

    /// Waits for one service to become ready.
    async fn wait_ready(&self) -> Result<(), DynError>;

    /// Checks whether one service is running.
    async fn is_running(&self) -> Result<bool, DynError>;
}

/// Runtime handles returned by one or more service deployment requests.
#[derive(Clone, Default)]
pub struct ContainerStackHandle {
    services: BTreeMap<String, ContainerServiceHandle>,
}

impl ContainerStackHandle {
    /// Creates a stack handle from backend-provisioned services.
    #[must_use]
    pub fn new(services: BTreeMap<String, ContainerServiceHandle>) -> Self {
        Self { services }
    }

    /// Returns one service by stable name.
    #[must_use]
    pub fn service(&self, name: &str) -> Option<&ContainerServiceHandle> {
        self.services.get(name)
    }

    /// Returns one service or a descriptive missing-service error.
    pub fn require_service(&self, name: &str) -> Result<&ContainerServiceHandle, DynError> {
        self.service(name)
            .ok_or_else(|| format!("service '{name}' is not available").into())
    }

    /// Adds handles returned by another deployment in the same backend session.
    pub fn merge(&mut self, other: Self) -> Result<(), DynError> {
        for name in other.services.keys() {
            if self.services.contains_key(name) {
                return Err(format!("service '{name}' is already available").into());
            }
        }
        self.services.extend(other.services);
        Ok(())
    }
}

/// A service stack handle paired with backend-owned cleanup.
pub struct ProvisionedContainerStack {
    handle: ContainerStackHandle,
    cleanup: Option<Box<dyn CleanupGuard>>,
}

impl ProvisionedContainerStack {
    /// Creates a provisioned stack.
    #[must_use]
    pub fn new(handle: ContainerStackHandle, cleanup: Option<Box<dyn CleanupGuard>>) -> Self {
        Self { handle, cleanup }
    }

    /// Returns the runtime handle.
    #[must_use]
    pub fn handle(&self) -> ContainerStackHandle {
        self.handle.clone()
    }

    /// Transfers backend cleanup ownership to the composition runtime.
    pub fn take_cleanup(&mut self) -> Option<Box<dyn CleanupGuard>> {
        self.cleanup.take()
    }
}

impl Drop for ProvisionedContainerStack {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup.cleanup();
        }
    }
}

/// Backend operation used by application deployments to provision services.
///
/// Compose is the first implementation. Kubernetes can implement the same
/// contract with Pods, Services, and ConfigMaps without changing app code.
/// Clones of one provisioner must share a deployment session so child apps can
/// be prepared separately while remaining mutually reachable.
#[async_trait]
pub trait ContainerStackProvisioner: Clone + Send + Sync + 'static {
    /// Adds one group of services to the active backend deployment session.
    async fn provision_container_stack(
        &self,
        request: ContainerStackRequest,
    ) -> Result<ProvisionedContainerStack, DynError>;
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use async_trait::async_trait;
    use testing_framework_core::scenario::{CleanupGuard, DynError};

    use super::{
        ContainerRestartPolicy, ContainerServiceControl, ContainerServiceHandle,
        ContainerServiceSpec, ContainerStackHandle, ProvisionedContainerStack,
    };

    struct NoopControl;

    #[async_trait]
    impl ContainerServiceControl for NoopControl {
        async fn start(&self) -> Result<(), DynError> {
            Ok(())
        }

        async fn stop(&self) -> Result<(), DynError> {
            Ok(())
        }

        async fn restart(&self) -> Result<(), DynError> {
            Ok(())
        }

        async fn wait_ready(&self) -> Result<(), DynError> {
            Ok(())
        }

        async fn is_running(&self) -> Result<bool, DynError> {
            Ok(true)
        }
    }

    struct CleanupProbe(Arc<AtomicBool>);

    impl CleanupGuard for CleanupProbe {
        fn cleanup(self: Box<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn handle(name: &str) -> ContainerStackHandle {
        ContainerStackHandle::new(
            [(
                name.to_owned(),
                ContainerServiceHandle::new(
                    name,
                    Default::default(),
                    Default::default(),
                    Arc::new(NoopControl),
                ),
            )]
            .into(),
        )
    }

    #[test]
    fn handles_from_separate_requests_merge_without_losing_services() {
        let mut services = handle("queue");
        services.merge(handle("worker")).unwrap();

        assert!(services.service("queue").is_some());
        assert!(services.service("worker").is_some());
        assert!(services.merge(handle("worker")).is_err());
    }

    #[test]
    fn unclaimed_provisioned_stack_runs_cleanup_on_drop() {
        let cleaned = Arc::new(AtomicBool::new(false));
        let unit = ProvisionedContainerStack::new(
            handle("worker"),
            Some(Box::new(CleanupProbe(Arc::clone(&cleaned)))),
        );

        drop(unit);

        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[test]
    fn services_do_not_hide_process_crashes_by_default() {
        let service = ContainerServiceSpec::new("worker", "worker:local");
        assert_eq!(service.restart_policy(), ContainerRestartPolicy::Never);

        let service = service.with_restart_policy(ContainerRestartPolicy::OnFailure);
        assert_eq!(service.restart_policy(), ContainerRestartPolicy::OnFailure);
    }
}
