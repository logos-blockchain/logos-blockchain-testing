use std::{
    env,
    path::{Path, PathBuf},
    process::Command as StdCommand,
    time::Duration,
};

use anyhow::anyhow;
use async_trait::async_trait;
use lb_framework::{
    NodeHttpClient,
    internal::{DeploymentPlan, NodePlan},
};
use lb_http_api_common::paths;
use reqwest::Url;
use testing_framework_core::{adjust_timeout, scenario::DynError};
use testing_framework_env as tf_env;
use testing_framework_runner_compose::{
    ComposeDeployEnv, ComposeDescriptor, ConfigServerHandle, EnvEntry, NodeDescriptor,
    docker::commands::run_docker_command,
    infrastructure::{ports::NodeHostPorts, template::repository_root},
};
use tokio::process::Command;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    LbcExtEnv,
    cfgsync::{CfgsyncOutputPaths, CfgsyncRenderOptions, render_and_write_cfgsync_from_template},
    constants::DEFAULT_CFGSYNC_PORT,
};

const NODE_ENTRYPOINT: &str = "/etc/logos/scripts/run_logos_node.sh";
const CFGSYNC_START_TIMEOUT: Duration = Duration::from_secs(180);
const DEFAULT_COMPOSE_RUNNER_HOST: &str = "127.0.0.1";
const DEFAULT_COMPOSE_TEST_IMAGE: &str = "logos-blockchain-testing:local";
const GHCR_TESTNET_IMAGE: &str = "ghcr.io/logos-co/nomos:testnet";
const DEFAULT_CFGSYNC_HOST: &str = "cfgsync";

#[derive(Debug)]
pub struct LbcCfgsyncHandle {
    name: String,
    stopped: bool,
}

impl LbcCfgsyncHandle {
    fn new(name: String) -> Self {
        Self {
            name,
            stopped: false,
        }
    }
}

impl ConfigServerHandle for LbcCfgsyncHandle {
    fn shutdown(&mut self) {
        if self.stopped {
            return;
        }
        let name = self.name.clone();
        let status = StdCommand::new("docker")
            .arg("rm")
            .arg("-f")
            .arg(&name)
            .status();
        match status {
            Ok(status) if status.success() => {
                debug!(container = name, "removed cfgsync container");
            }
            Ok(status) => {
                warn!(container = name, status = ?status, "failed to remove cfgsync container");
            }
            Err(err) => {
                warn!(container = name, error = ?err, "failed to spawn docker rm for cfgsync container");
            }
        }
        self.stopped = true;
    }

    fn mark_preserved(&mut self) {
        self.stopped = true;
    }

    fn container_name(&self) -> Option<&str> {
        Some(self.name.as_str())
    }
}

#[async_trait]
impl ComposeDeployEnv for LbcExtEnv {
    type ConfigHandle = LbcCfgsyncHandle;

    fn compose_descriptor(topology: &Self::Deployment, cfgsync_port: u16) -> ComposeDescriptor {
        let cfgsync_port = normalized_cfgsync_port(cfgsync_port);
        let (image, platform) = resolve_image();
        let nodes = topology
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| {
                build_compose_node_descriptor(index, node, cfgsync_port, &image, platform.clone())
            })
            .collect();

        ComposeDescriptor::new(nodes)
    }

    fn update_cfgsync_config(
        path: &Path,
        topology: &Self::Deployment,
        port: u16,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError> {
        debug!(
            path = %path.display(),
            port,
            nodes = topology.nodes().len(),
            "updating cfgsync template"
        );
        let artifacts_path = cfgsync_artifacts_path(path);
        let hostnames = topology_hostnames(topology);
        let options = cfgsync_render_options(port, metrics_otlp_ingest_url);

        render_and_write_cfgsync_from_template::<lb_framework::LbcEnv>(
            topology,
            &hostnames,
            options,
            CfgsyncOutputPaths {
                config_path: path,
                artifacts_path: &artifacts_path,
            },
        )?;
        Ok(())
    }

    async fn start_cfgsync(
        cfgsync_path: &Path,
        port: u16,
        network: &str,
    ) -> Result<Self::ConfigHandle, DynError> {
        let testnet_dir = cfgsync_dir(cfgsync_path)?;
        let (image, _) = resolve_image();
        let container_name = cfgsync_container_name();

        debug!(
            container = %container_name,
            image,
            cfgsync = %cfgsync_path.display(),
            port,
            "starting cfgsync container"
        );

        let command =
            build_cfgsync_docker_run_command(&container_name, network, port, testnet_dir, &image);

        run_docker_command(
            command,
            adjust_timeout(CFGSYNC_START_TIMEOUT),
            "docker run cfgsync server",
        )
        .await
        .map_err(|err| anyhow!(err.to_string()))?;

        info!(container = %container_name, port, "cfgsync container started");

        Ok(LbcCfgsyncHandle::new(container_name))
    }

    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError> {
        api_client_from_host_ports(ports, host)
    }

    fn readiness_path() -> &'static str {
        paths::CRYPTARCHIA_INFO
    }

    fn compose_runner_host() -> String {
        compose_runner_host()
    }
}

fn node_instance_name(index: usize) -> String {
    format!("node-{index}")
}

fn cfgsync_artifacts_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or(config_path)
        .join("cfgsync.artifacts.yaml")
}

fn topology_hostnames(topology: &DeploymentPlan) -> Vec<String> {
    topology
        .nodes()
        .iter()
        .map(|node| format!("node-{}", node.index()))
        .collect()
}

fn cfgsync_render_options(
    port: u16,
    metrics_otlp_ingest_url: Option<&Url>,
) -> CfgsyncRenderOptions {
    CfgsyncRenderOptions {
        port: Some(port),
        artifacts_path: None,
        min_timeout_secs: None,
        metrics_otlp_ingest_url: metrics_otlp_ingest_url.cloned(),
    }
}

fn cfgsync_dir(cfgsync_path: &Path) -> Result<&Path, DynError> {
    cfgsync_path
        .parent()
        .ok_or_else(|| anyhow!("cfgsync path {cfgsync_path:?} has no parent directory").into())
}

fn normalized_cfgsync_port(port: u16) -> u16 {
    if port == 0 {
        DEFAULT_CFGSYNC_PORT
    } else {
        port
    }
}

fn build_compose_node_descriptor(
    index: usize,
    node: &NodePlan,
    cfgsync_port: u16,
    image: &str,
    platform: Option<String>,
) -> NodeDescriptor {
    let mut environment = base_environment(cfgsync_port);
    environment.push(EnvEntry::new(
        "CFG_HOST_IDENTIFIER",
        node_instance_name(index),
    ));

    let api_port = node.general.api_config.address.port();
    let testing_port = node.general.api_config.testing_http_address.port();
    let ports = vec![
        format!("127.0.0.1::{api_port}"),
        format!("127.0.0.1::{testing_port}"),
    ];

    NodeDescriptor::new(
        node_instance_name(index),
        image.to_owned(),
        NODE_ENTRYPOINT,
        base_volumes(),
        default_extra_hosts(),
        ports,
        environment,
        platform,
    )
}

fn cfgsync_container_name() -> String {
    format!("nomos-cfgsync-{}", Uuid::new_v4())
}

fn cfgsync_stack_volume_arg(testnet_dir: &Path) -> String {
    let stack_dir = testnet_dir
        .canonicalize()
        .unwrap_or_else(|_| testnet_dir.to_path_buf());
    format!("{}:/etc/logos:ro", stack_dir.display())
}

fn maybe_add_circuits_mount(command: &mut Command) {
    let circuits_dir = env::var("LOGOS_BLOCKCHAIN_CIRCUITS_DOCKER")
        .ok()
        .or_else(|| env::var("LOGOS_BLOCKCHAIN_CIRCUITS").ok());

    let Some(circuits_dir) = circuits_dir else {
        return;
    };

    let host_path = PathBuf::from(&circuits_dir);
    if !host_path.exists() {
        return;
    }

    let resolved_host_path = host_path.canonicalize().unwrap_or(host_path);
    command
        .arg("-e")
        .arg(format!("LOGOS_BLOCKCHAIN_CIRCUITS={circuits_dir}"))
        .arg("-v")
        .arg(format!(
            "{}:{circuits_dir}:ro",
            resolved_host_path.display()
        ));
}

fn build_cfgsync_docker_run_command(
    container_name: &str,
    network: &str,
    port: u16,
    testnet_dir: &Path,
    image: &str,
) -> Command {
    let mut command = Command::new("docker");
    command
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(container_name)
        .arg("--network")
        .arg(network)
        .arg("--network-alias")
        .arg("cfgsync")
        .arg("--workdir")
        .arg("/etc/logos")
        .arg("--entrypoint")
        .arg("cfgsync-server")
        .arg("-p")
        .arg(format!("{port}:{port}"))
        .arg("-v")
        .arg(cfgsync_stack_volume_arg(testnet_dir));

    maybe_add_circuits_mount(&mut command);
    command.arg(image).arg("/etc/logos/cfgsync.yaml");
    command
}

fn resolve_image() -> (String, Option<String>) {
    let image =
        tf_env::nomos_testnet_image().unwrap_or_else(|| String::from(DEFAULT_COMPOSE_TEST_IMAGE));
    let platform = (image == GHCR_TESTNET_IMAGE).then(|| "linux/amd64".to_owned());
    debug!(image, platform = ?platform, "resolved compose image");
    (image, platform)
}

fn base_volumes() -> Vec<String> {
    let mut volumes = vec!["./stack:/etc/logos".into()];
    if let Some(host_log_dir) = repository_root()
        .ok()
        .map(|root| root.join("tmp").join("node-logs"))
        .map(|dir| dir.display().to_string())
    {
        volumes.push(format!("{host_log_dir}:/tmp/node-logs"));
    }
    volumes
}

fn default_extra_hosts() -> Vec<String> {
    testing_framework_runner_compose::docker::platform::host_gateway_entry()
        .into_iter()
        .collect()
}

fn base_environment(cfgsync_port: u16) -> Vec<EnvEntry> {
    let rust_log = env_value_or_default(tf_env::rust_log, "info");
    let nomos_log_level = env_value_or_default(tf_env::nomos_log_level, "info");
    let time_backend = env_value_or_default(tf_env::lb_time_service_backend, "monotonic");
    let cfgsync_host = env::var("LOGOS_BLOCKCHAIN_CFGSYNC_HOST")
        .unwrap_or_else(|_| String::from(DEFAULT_CFGSYNC_HOST));
    vec![
        EnvEntry::new("RUST_LOG", rust_log),
        EnvEntry::new("LOGOS_BLOCKCHAIN_LOG_LEVEL", nomos_log_level),
        EnvEntry::new("LOGOS_BLOCKCHAIN_TIME_BACKEND", time_backend),
        EnvEntry::new(
            "CFG_SERVER_ADDR",
            format!("http://{cfgsync_host}:{cfgsync_port}"),
        ),
        EnvEntry::new("OTEL_METRIC_EXPORT_INTERVAL", "5000"),
    ]
}

fn compose_runner_host() -> String {
    env::var("COMPOSE_RUNNER_HOST").unwrap_or_else(|_| DEFAULT_COMPOSE_RUNNER_HOST.to_string())
}

fn api_client_from_host_ports(
    ports: &NodeHostPorts,
    host: &str,
) -> Result<NodeHttpClient, DynError> {
    let base_url = url_for_host_port(host, ports.api)?;
    let testing_url = url_for_host_port(host, ports.testing)?;
    Ok(NodeHttpClient::from_urls(base_url, Some(testing_url)))
}

fn env_value_or_default(getter: impl Fn() -> Option<String>, default: &'static str) -> String {
    getter().unwrap_or_else(|| String::from(default))
}

fn url_for_host_port(host: &str, port: u16) -> Result<Url, DynError> {
    let url = Url::parse(&format!("http://{host}:{port}/"))?;
    Ok(url)
}
