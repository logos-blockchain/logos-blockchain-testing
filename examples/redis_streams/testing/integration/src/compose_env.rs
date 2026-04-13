use std::{env, fs, io::Error, path::Path};

use testing_framework_core::{
    cfgsync::StaticArtifactRenderer, scenario::DynError, topology::DeploymentDescriptor,
};
use testing_framework_runner_compose::{
    ComposeDeployEnv, ComposeNodeConfigFileName, ComposeReadinessProbe, EnvEntry,
    LoopbackNodeRuntimeSpec, infrastructure::ports::NodeHostPorts,
};

use crate::{RedisStreamsClient, RedisStreamsEnv};

const NODE_CONFIG_PATH: &str = "/usr/local/etc/redis/redis.conf";
const REDIS_PORT: u16 = 6379;

impl ComposeDeployEnv for RedisStreamsEnv {
    fn prepare_compose_configs(
        path: &Path,
        topology: &Self::Deployment,
        _cfgsync_port: u16,
        _metrics_otlp_ingest_url: Option<&reqwest::Url>,
    ) -> Result<(), DynError> {
        let hostnames = Self::cfgsync_hostnames(topology);
        let configs_dir = path
            .parent()
            .ok_or_else(|| Error::other("cfgsync path has no parent"))?
            .join("configs");
        fs::create_dir_all(&configs_dir)?;

        for index in 0..topology.node_count() {
            let mut config = <Self as StaticArtifactRenderer>::build_node_config(topology, index)?;
            <Self as StaticArtifactRenderer>::rewrite_for_hostnames(
                topology,
                index,
                &hostnames,
                &mut config,
            )?;
            let rendered = <Self as StaticArtifactRenderer>::serialize_node_config(&config)?;
            fs::write(
                configs_dir.join(Self::static_node_config_file_name(index)),
                rendered,
            )?;
        }

        Ok(())
    }

    fn static_node_config_file_name(index: usize) -> String {
        ComposeNodeConfigFileName::FixedExtension("conf").resolve(index)
    }

    fn loopback_node_runtime_spec(
        _topology: &Self::Deployment,
        index: usize,
    ) -> Result<Option<LoopbackNodeRuntimeSpec>, DynError> {
        Ok(Some(build_redis_runtime(index)))
    }

    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError> {
        redis_client_from_ports(ports, host)
    }

    fn readiness_probe() -> ComposeReadinessProbe {
        ComposeReadinessProbe::Tcp
    }
}

fn build_redis_runtime(index: usize) -> LoopbackNodeRuntimeSpec {
    let image = env::var("REDIS_STREAMS_IMAGE").unwrap_or_else(|_| "redis:7".to_owned());
    let platform = env::var("REDIS_STREAMS_PLATFORM").ok();
    LoopbackNodeRuntimeSpec {
        image,
        entrypoint: build_redis_entrypoint(),
        volumes: vec![format!(
            "./stack/configs/node-{index}.conf:{NODE_CONFIG_PATH}:ro"
        )],
        extra_hosts: vec![],
        container_ports: vec![REDIS_PORT],
        environment: vec![EnvEntry::new("RUST_LOG", "info")],
        platform,
    }
}

fn redis_client_from_ports(
    ports: &NodeHostPorts,
    host: &str,
) -> Result<RedisStreamsClient, DynError> {
    RedisStreamsClient::new(format!("redis://{host}:{}", ports.api))
}

fn build_redis_entrypoint() -> Vec<String> {
    vec![
        "redis-server".to_owned(),
        NODE_CONFIG_PATH.to_owned(),
        "--bind".to_owned(),
        "0.0.0.0".to_owned(),
        "--port".to_owned(),
        REDIS_PORT.to_string(),
        "--protected-mode".to_owned(),
        "no".to_owned(),
    ]
}
