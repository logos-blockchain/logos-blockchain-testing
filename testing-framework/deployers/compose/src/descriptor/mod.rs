use serde::Serialize;
use testing_framework_core::{
    constants::{DEFAULT_CFGSYNC_PORT, DEFAULT_PROMETHEUS_HTTP_PORT, kzg_container_path},
    topology::generation::{GeneratedNodeConfig, GeneratedTopology},
};

use crate::docker::platform::{host_gateway_entry, resolve_image};

mod node;

pub use node::{EnvEntry, NodeDescriptor};

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
    grafana: GrafanaTemplate,
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
    pub fn validators(&self) -> &[NodeDescriptor] {
        &self.validators
    }

    #[cfg(test)]
    pub fn executors(&self) -> &[NodeDescriptor] {
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
    grafana_port: Option<u16>,
}

impl<'a> ComposeDescriptorBuilder<'a> {
    const fn new(topology: &'a GeneratedTopology) -> Self {
        Self {
            topology,
            use_kzg_mount: false,
            cfgsync_port: None,
            prometheus_port: None,
            grafana_port: None,
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

    #[must_use]
    /// Set host port mapping for Grafana.
    pub const fn with_grafana_port(mut self, port: u16) -> Self {
        self.grafana_port = Some(port);
        self
    }

    /// Finish building the descriptor, erroring if required fields are missing.
    pub fn build(self) -> Result<ComposeDescriptor, DescriptorBuildError> {
        let cfgsync_port = self.cfgsync_port.unwrap_or(DEFAULT_CFGSYNC_PORT);
        let prometheus_host_port = self
            .prometheus_port
            .ok_or(DescriptorBuildError::MissingPrometheusPort)?;
        let grafana_host_port = self.grafana_port.unwrap_or(0);

        let (image, platform) = resolve_image();
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
            grafana: GrafanaTemplate::new(grafana_host_port),
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
            host_port: format!("127.0.0.1:{port}:{}", DEFAULT_PROMETHEUS_HTTP_PORT),
            platform,
        }
    }
}

/// Minimal Grafana service mapping used in the compose template.
#[derive(Clone, Debug, Serialize)]
pub struct GrafanaTemplate {
    host_port: String,
}

impl GrafanaTemplate {
    fn new(port: u16) -> Self {
        let host_port = match port {
            0 => "127.0.0.1::3000".to_string(), // docker assigns host port
            _ => format!("127.0.0.1:{port}:3000"),
        };

        Self { host_port }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ComposeNodeKind {
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
