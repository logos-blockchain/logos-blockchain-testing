use std::{
    env,
    path::{Path, PathBuf},
};

use serde::Serialize;
use testing_framework_core::topology::generation::{GeneratedNodeConfig, GeneratedTopology};
use testing_framework_env as tf_env;

use crate::docker::platform::{host_gateway_entry, resolve_image};

mod node;

pub use node::{EnvEntry, NodeDescriptor};
use testing_framework_config::constants::DEFAULT_CFGSYNC_PORT;

/// Top-level docker-compose descriptor built from a GeneratedTopology.
#[derive(Clone, Debug, Serialize)]
pub struct ComposeDescriptor {
    nodes: Vec<NodeDescriptor>,
}

impl ComposeDescriptor {
    /// Start building a descriptor from a generated topology.
    #[must_use]
    pub const fn builder(topology: &GeneratedTopology) -> ComposeDescriptorBuilder<'_> {
        ComposeDescriptorBuilder::new(topology)
    }

    #[cfg(test)]
    pub fn nodes(&self) -> &[NodeDescriptor] {
        &self.nodes
    }
}

/// Builder for `ComposeDescriptor` that plugs topology values into the
/// template.
pub struct ComposeDescriptorBuilder<'a> {
    topology: &'a GeneratedTopology,
    cfgsync_port: Option<u16>,
}

impl<'a> ComposeDescriptorBuilder<'a> {
    const fn new(topology: &'a GeneratedTopology) -> Self {
        Self {
            topology,
            cfgsync_port: None,
        }
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

        let nodes = build_nodes(
            self.topology.nodes(),
            &image,
            platform.as_deref(),
            cfgsync_port,
        );

        ComposeDescriptor { nodes }
    }
}

const NODE_ENTRYPOINT: &str = "/etc/nomos/scripts/run_nomos_node.sh";

pub(crate) fn node_instance_name(index: usize) -> String {
    format!("node-{index}")
}

fn build_nodes(
    nodes: &[GeneratedNodeConfig],
    image: &str,
    platform: Option<&str>,
    cfgsync_port: u16,
) -> Vec<NodeDescriptor> {
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| NodeDescriptor::from_node(index, node, image, platform, cfgsync_port))
        .collect()
}

fn base_volumes() -> Vec<String> {
    let mut volumes = vec!["./stack:/etc/nomos".into()];
    if let Some(host_log_dir) = repo_root()
        .map(|root| root.join("tmp").join("node-logs"))
        .map(|dir| dir.display().to_string())
    {
        volumes.push(format!("{host_log_dir}:/tmp/node-logs"));
    }
    volumes
}

fn repo_root() -> Option<PathBuf> {
    if let Ok(root) = env::var("CARGO_WORKSPACE_DIR") {
        return Some(PathBuf::from(root));
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn default_extra_hosts() -> Vec<String> {
    host_gateway_entry().into_iter().collect()
}

fn base_environment(cfgsync_port: u16) -> Vec<EnvEntry> {
    let pol_mode = tf_env::pol_proof_dev_mode().unwrap_or_else(|| "true".to_string());
    let rust_log = tf_env::rust_log().unwrap_or_else(|| "info".to_string());
    let nomos_log_level = tf_env::nomos_log_level().unwrap_or_else(|| "info".to_string());
    let time_backend = tf_env::nomos_time_backend().unwrap_or_else(|| "monotonic".into());
    vec![
        EnvEntry::new("POL_PROOF_DEV_MODE", pol_mode),
        EnvEntry::new("RUST_LOG", rust_log),
        EnvEntry::new("LOGOS_BLOCKCHAIN_LOG_LEVEL", nomos_log_level),
        EnvEntry::new("LOGOS_BLOCKCHAIN_TIME_BACKEND", time_backend),
        EnvEntry::new(
            "CFG_SERVER_ADDR",
            format!("http://host.docker.internal:{cfgsync_port}"),
        ),
        EnvEntry::new("OTEL_METRIC_EXPORT_INTERVAL", "5000"),
    ]
}
