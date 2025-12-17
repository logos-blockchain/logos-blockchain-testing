use serde::Serialize;
use testing_framework_core::{
    constants::{DEFAULT_CFGSYNC_PORT, kzg_container_path},
    topology::generation::{GeneratedNodeConfig, GeneratedTopology},
};

use crate::docker::platform::{host_gateway_entry, resolve_image};

mod node;

pub use node::{EnvEntry, NodeDescriptor};

/// Top-level docker-compose descriptor built from a GeneratedTopology.
#[derive(Clone, Debug, Serialize)]
pub struct ComposeDescriptor {
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
}

impl<'a> ComposeDescriptorBuilder<'a> {
    const fn new(topology: &'a GeneratedTopology) -> Self {
        Self {
            topology,
            use_kzg_mount: false,
            cfgsync_port: None,
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

    /// Finish building the descriptor.
    #[must_use]
    pub fn build(self) -> ComposeDescriptor {
        let cfgsync_port = self.cfgsync_port.unwrap_or(DEFAULT_CFGSYNC_PORT);

        let (image, platform) = resolve_image();

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

        ComposeDescriptor {
            validators,
            executors,
        }
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
