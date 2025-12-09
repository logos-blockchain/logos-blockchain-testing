use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process,
    time::Duration,
};

use anyhow::Context as _;
use serde::Serialize;
use tera::Context as TeraContext;
use testing_framework_core::{
    adjust_timeout,
    constants::{DEFAULT_CFGSYNC_PORT, kzg_container_path},
    topology::{GeneratedNodeConfig, GeneratedTopology},
};
use tokio::{process::Command, time::timeout};

const COMPOSE_UP_TIMEOUT: Duration = Duration::from_secs(120);
const TEMPLATE_RELATIVE_PATH: &str =
    "testing-framework/runners/compose/assets/docker-compose.yml.tera";

/// Errors running docker compose commands.
#[derive(Debug, thiserror::Error)]
pub enum ComposeCommandError {
    #[error("{command} exited with status {status}")]
    Failed {
        command: String,
        status: process::ExitStatus,
    },
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} timed out after {timeout:?}")]
    Timeout { command: String, timeout: Duration },
}

/// Runs `docker compose up -d` for the generated stack.
pub async fn compose_up(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("up")
        .arg("-d")
        .current_dir(root);

    run_compose_command(cmd, adjust_timeout(COMPOSE_UP_TIMEOUT), "docker compose up").await
}

/// Runs `docker compose down --volumes` for the generated stack.
pub async fn compose_down(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("down")
        .arg("--volumes")
        .current_dir(root);

    run_compose_command(
        cmd,
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose down",
    )
    .await
}

/// Errors when templating docker-compose files.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("failed to resolve repository root for compose template: {source}")]
    RepositoryRoot {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to read compose template at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to serialise compose descriptor for templating: {source}")]
    Serialize {
        #[source]
        source: tera::Error,
    },
    #[error("failed to render compose template at {path}: {source}")]
    Render {
        path: PathBuf,
        #[source]
        source: tera::Error,
    },
    #[error("failed to write compose file at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Errors building a compose descriptor from the topology.
#[derive(Debug, thiserror::Error)]
pub enum DescriptorBuildError {
    #[error("prometheus port is not configured for compose descriptor")]
    MissingPrometheusPort,
}

/// Top-level docker-compose descriptor built from a GeneratedTopology.
#[derive(Clone, Debug, Serialize)]
pub struct ComposeDescriptor {
    prometheus: PrometheusTemplate,
    validators: Vec<NodeDescriptor>,
    executors: Vec<NodeDescriptor>,
}

impl ComposeDescriptor {
    /// Start building a descriptor from a generated topology.
    #[must_use]
    pub const fn builder(topology: &GeneratedTopology) -> ComposeDescriptorBuilder<'_> {
        ComposeDescriptorBuilder::new(topology)
    }

    #[cfg(test)]
    fn validators(&self) -> &[NodeDescriptor] {
        &self.validators
    }

    #[cfg(test)]
    fn executors(&self) -> &[NodeDescriptor] {
        &self.executors
    }
}

/// Builder for `ComposeDescriptor` that plugs topology values into the
/// template.
pub struct ComposeDescriptorBuilder<'a> {
    topology: &'a GeneratedTopology,
    use_kzg_mount: bool,
    cfgsync_port: Option<u16>,
    prometheus_port: Option<u16>,
}

impl<'a> ComposeDescriptorBuilder<'a> {
    const fn new(topology: &'a GeneratedTopology) -> Self {
        Self {
            topology,
            use_kzg_mount: false,
            cfgsync_port: None,
            prometheus_port: None,
        }
    }

    #[must_use]
    /// Mount KZG parameters into nodes when enabled.
    pub const fn with_kzg_mount(mut self, enabled: bool) -> Self {
        self.use_kzg_mount = enabled;
        self
    }

    #[must_use]
    /// Set cfgsync port for nodes.
    pub const fn with_cfgsync_port(mut self, port: u16) -> Self {
        self.cfgsync_port = Some(port);
        self
    }

    #[must_use]
    /// Set host port mapping for Prometheus.
    pub const fn with_prometheus_port(mut self, port: u16) -> Self {
        self.prometheus_port = Some(port);
        self
    }

    /// Finish building the descriptor, erroring if required fields are missing.
    pub fn build(self) -> Result<ComposeDescriptor, DescriptorBuildError> {
        let cfgsync_port = self.cfgsync_port.unwrap_or(DEFAULT_CFGSYNC_PORT);
        let prometheus_host_port = self
            .prometheus_port
            .ok_or(DescriptorBuildError::MissingPrometheusPort)?;

        let (default_image, default_platform) = resolve_image();
        let image = default_image;
        let platform = default_platform;
        // Prometheus image is x86_64-only on some tags; set platform when on arm hosts.
        let prometheus_platform = match std::env::consts::ARCH {
            "aarch64" | "arm64" => Some(String::from("linux/arm64")),
            _ => None,
        };

        let validators = build_nodes(
            self.topology.validators(),
            ComposeNodeKind::Validator,
            &image,
            platform.as_deref(),
            self.use_kzg_mount,
            cfgsync_port,
        );

        let executors = build_nodes(
            self.topology.executors(),
            ComposeNodeKind::Executor,
            &image,
            platform.as_deref(),
            self.use_kzg_mount,
            cfgsync_port,
        );

        Ok(ComposeDescriptor {
            prometheus: PrometheusTemplate::new(prometheus_host_port, prometheus_platform),
            validators,
            executors,
        })
    }
}

/// Minimal Prometheus service mapping used in the compose template.
#[derive(Clone, Debug, Serialize)]
pub struct PrometheusTemplate {
    host_port: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

impl PrometheusTemplate {
    fn new(port: u16, platform: Option<String>) -> Self {
        Self {
            host_port: format!("127.0.0.1:{port}:9090"),
            platform,
        }
    }
}

/// Environment variable entry for docker-compose templating.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EnvEntry {
    key: String,
    value: String,
}

impl EnvEntry {
    fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    #[cfg(test)]
    fn key(&self) -> &str {
        &self.key
    }

    #[cfg(test)]
    fn value(&self) -> &str {
        &self.value
    }
}

/// Describes a validator or executor container in the compose stack.
#[derive(Clone, Debug, Serialize)]
pub struct NodeDescriptor {
    name: String,
    image: String,
    entrypoint: String,
    volumes: Vec<String>,
    extra_hosts: Vec<String>,
    ports: Vec<String>,
    environment: Vec<EnvEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

/// Host ports mapped for a single node.
#[derive(Clone, Debug)]
pub struct NodeHostPorts {
    pub api: u16,
    pub testing: u16,
}

/// All host port mappings for validators and executors.
#[derive(Clone, Debug)]
pub struct HostPortMapping {
    pub validators: Vec<NodeHostPorts>,
    pub executors: Vec<NodeHostPorts>,
}

impl HostPortMapping {
    /// Returns API ports for all validators.
    pub fn validator_api_ports(&self) -> Vec<u16> {
        self.validators.iter().map(|ports| ports.api).collect()
    }

    /// Returns API ports for all executors.
    pub fn executor_api_ports(&self) -> Vec<u16> {
        self.executors.iter().map(|ports| ports.api).collect()
    }
}

impl NodeDescriptor {
    fn from_node(
        kind: ComposeNodeKind,
        index: usize,
        node: &GeneratedNodeConfig,
        image: &str,
        platform: Option<&str>,
        use_kzg_mount: bool,
        cfgsync_port: u16,
    ) -> Self {
        let mut environment = base_environment(cfgsync_port);
        let identifier = kind.instance_name(index);
        environment.extend([
            EnvEntry::new(
                "CFG_NETWORK_PORT",
                node.general.network_config.backend.inner.port.to_string(),
            ),
            EnvEntry::new("CFG_DA_PORT", node.da_port.to_string()),
            EnvEntry::new("CFG_BLEND_PORT", node.blend_port.to_string()),
            EnvEntry::new(
                "CFG_API_PORT",
                node.general.api_config.address.port().to_string(),
            ),
            EnvEntry::new(
                "CFG_TESTING_HTTP_PORT",
                node.general
                    .api_config
                    .testing_http_address
                    .port()
                    .to_string(),
            ),
            EnvEntry::new("CFG_HOST_IDENTIFIER", identifier),
        ]);

        let ports = vec![
            node.general.api_config.address.port().to_string(),
            node.general
                .api_config
                .testing_http_address
                .port()
                .to_string(),
        ];

        Self {
            name: kind.instance_name(index),
            image: image.to_owned(),
            entrypoint: kind.entrypoint().to_owned(),
            volumes: base_volumes(use_kzg_mount),
            extra_hosts: default_extra_hosts(),
            ports,
            environment,
            platform: platform.map(ToOwned::to_owned),
        }
    }

    #[cfg(test)]
    fn ports(&self) -> &[String] {
        &self.ports
    }

    #[cfg(test)]
    fn environment(&self) -> &[EnvEntry] {
        &self.environment
    }
}

/// Render and write the compose file to disk.
pub fn write_compose_file(
    descriptor: &ComposeDescriptor,
    compose_path: &Path,
) -> Result<(), TemplateError> {
    TemplateSource::load()?.write(descriptor, compose_path)
}

/// Dump docker compose logs to stderr for debugging failures.
pub async fn dump_compose_logs(compose_file: &Path, project: &str, root: &Path) {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project)
        .arg("logs")
        .arg("--no-color")
        .current_dir(root);

    match cmd.output().await {
        Ok(output) => print_logs(&output.stdout, &output.stderr),
        Err(err) => eprintln!("[compose-runner] failed to collect docker compose logs: {err}"),
    }
}

fn print_logs(stdout: &[u8], stderr: &[u8]) {
    if !stdout.is_empty() {
        eprintln!(
            "[compose-runner] docker compose logs:\n{}",
            String::from_utf8_lossy(stdout)
        );
    }
    if !stderr.is_empty() {
        eprintln!(
            "[compose-runner] docker compose errors:\n{}",
            String::from_utf8_lossy(stderr)
        );
    }
}

struct TemplateSource {
    path: PathBuf,
    contents: String,
}

impl TemplateSource {
    fn load() -> Result<Self, TemplateError> {
        let repo_root =
            repository_root().map_err(|source| TemplateError::RepositoryRoot { source })?;
        let path = repo_root.join(TEMPLATE_RELATIVE_PATH);
        let contents = fs::read_to_string(&path).map_err(|source| TemplateError::Read {
            path: path.clone(),
            source,
        })?;

        Ok(Self { path, contents })
    }

    fn render(&self, descriptor: &ComposeDescriptor) -> Result<String, TemplateError> {
        let context = TeraContext::from_serialize(descriptor)
            .map_err(|source| TemplateError::Serialize { source })?;

        tera::Tera::one_off(&self.contents, &context, false).map_err(|source| {
            TemplateError::Render {
                path: self.path.clone(),
                source,
            }
        })
    }

    fn write(&self, descriptor: &ComposeDescriptor, output: &Path) -> Result<(), TemplateError> {
        let rendered = self.render(descriptor)?;
        fs::write(output, rendered).map_err(|source| TemplateError::Write {
            path: output.to_path_buf(),
            source,
        })
    }
}

/// Resolve the repository root, respecting `CARGO_WORKSPACE_DIR` override.
pub fn repository_root() -> anyhow::Result<PathBuf> {
    env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .map(PathBuf::from)
                .context("resolving repository root from manifest dir")
        })
}

#[derive(Clone, Copy)]
enum ComposeNodeKind {
    Validator,
    Executor,
}

impl ComposeNodeKind {
    fn instance_name(self, index: usize) -> String {
        match self {
            Self::Validator => format!("validator-{index}"),
            Self::Executor => format!("executor-{index}"),
        }
    }

    const fn entrypoint(self) -> &'static str {
        match self {
            Self::Validator => "/etc/nomos/scripts/run_nomos_node.sh",
            Self::Executor => "/etc/nomos/scripts/run_nomos_executor.sh",
        }
    }
}

fn build_nodes(
    nodes: &[GeneratedNodeConfig],
    kind: ComposeNodeKind,
    image: &str,
    platform: Option<&str>,
    use_kzg_mount: bool,
    cfgsync_port: u16,
) -> Vec<NodeDescriptor> {
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            NodeDescriptor::from_node(
                kind,
                index,
                node,
                image,
                platform,
                use_kzg_mount,
                cfgsync_port,
            )
        })
        .collect()
}

fn base_environment(cfgsync_port: u16) -> Vec<EnvEntry> {
    let pol_mode = std::env::var("POL_PROOF_DEV_MODE").unwrap_or_else(|_| "true".to_string());
    let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let nomos_log_level = std::env::var("NOMOS_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let time_backend = std::env::var("NOMOS_TIME_BACKEND").unwrap_or_else(|_| "monotonic".into());
    let kzg_path =
        std::env::var("NOMOS_KZGRS_PARAMS_PATH").unwrap_or_else(|_| kzg_container_path());
    vec![
        EnvEntry::new("POL_PROOF_DEV_MODE", pol_mode),
        EnvEntry::new("RUST_LOG", rust_log),
        EnvEntry::new("NOMOS_LOG_LEVEL", nomos_log_level),
        EnvEntry::new("NOMOS_TIME_BACKEND", time_backend),
        EnvEntry::new("NOMOS_KZGRS_PARAMS_PATH", kzg_path),
        EnvEntry::new(
            "CFG_SERVER_ADDR",
            format!("http://host.docker.internal:{cfgsync_port}"),
        ),
        EnvEntry::new("OTEL_METRIC_EXPORT_INTERVAL", "5000"),
    ]
}

fn base_volumes(use_kzg_mount: bool) -> Vec<String> {
    let mut volumes = vec!["./stack:/etc/nomos".into()];
    if use_kzg_mount {
        volumes.push("./kzgrs_test_params:/kzgrs_test_params:z".into());
    }
    volumes
}

fn default_extra_hosts() -> Vec<String> {
    host_gateway_entry().into_iter().collect()
}

/// Select the compose image and optional platform, honoring
/// NOMOS_TESTNET_IMAGE.
pub fn resolve_image() -> (String, Option<String>) {
    let image =
        env::var("NOMOS_TESTNET_IMAGE").unwrap_or_else(|_| String::from("nomos-testnet:local"));
    let platform = (image == "ghcr.io/logos-co/nomos:testnet").then(|| "linux/amd64".to_owned());
    (image, platform)
}

fn host_gateway_entry() -> Option<String> {
    if let Ok(value) = env::var("COMPOSE_RUNNER_HOST_GATEWAY") {
        if value.eq_ignore_ascii_case("disable") || value.is_empty() {
            return None;
        }
        return Some(value);
    }

    if let Ok(gateway) = env::var("DOCKER_HOST_GATEWAY") {
        if !gateway.is_empty() {
            return Some(format!("host.docker.internal:{gateway}"));
        }
    }

    Some("host.docker.internal:host-gateway".into())
}

async fn run_compose_command(
    mut command: Command,
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    let result = timeout(timeout_duration, command.status()).await;
    match result {
        Ok(status) => handle_compose_status(status, description),
        Err(_) => Err(ComposeCommandError::Timeout {
            command: description.to_owned(),
            timeout: timeout_duration,
        }),
    }
}

fn handle_compose_status(
    status: std::io::Result<std::process::ExitStatus>,
    description: &str,
) -> Result<(), ComposeCommandError> {
    match status {
        Ok(code) if code.success() => Ok(()),
        Ok(code) => Err(ComposeCommandError::Failed {
            command: description.to_owned(),
            status: code,
        }),
        Err(err) => Err(ComposeCommandError::Spawn {
            command: description.to_owned(),
            source: err,
        }),
    }
}

#[cfg(test)]
mod tests {
    use testing_framework_core::topology::{TopologyBuilder, TopologyConfig};

    use super::*;

    #[test]
    fn descriptor_matches_topology_counts() {
        let topology = TopologyBuilder::new(TopologyConfig::with_node_numbers(2, 1)).build();
        let descriptor = ComposeDescriptor::builder(&topology)
            .with_cfgsync_port(DEFAULT_CFGSYNC_PORT)
            .with_prometheus_port(9090)
            .build()
            .expect("descriptor");

        assert_eq!(descriptor.validators().len(), topology.validators().len());
        assert_eq!(descriptor.executors().len(), topology.executors().len());
    }

    #[test]
    fn descriptor_includes_expected_env_and_ports() {
        let topology = TopologyBuilder::new(TopologyConfig::with_node_numbers(1, 1)).build();
        let cfgsync_port = 4555;
        let descriptor = ComposeDescriptor::builder(&topology)
            .with_cfgsync_port(cfgsync_port)
            .with_prometheus_port(9090)
            .build()
            .expect("descriptor");

        let validator = &descriptor.validators()[0];
        assert!(
            validator
                .environment()
                .iter()
                .any(|entry| entry.key() == "CFG_SERVER_ADDR"
                    && entry.value() == format!("http://host.docker.internal:{cfgsync_port}"))
        );

        let api_container = topology.validators()[0].general.api_config.address.port();
        assert!(validator.ports().contains(&api_container.to_string()));
    }
}
