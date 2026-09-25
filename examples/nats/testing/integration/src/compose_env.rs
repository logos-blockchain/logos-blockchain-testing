use std::{env, fs, io::Error, path::Path};

use testing_framework_core::{
    cfgsync::StaticArtifactRenderer, scenario::DynError, topology::DeploymentDescriptor,
};
use testing_framework_runner_compose::{
    ComposeDeployEnv, ComposeNodeConfigFileName, LoopbackNodeRuntimeSpec, node_identifier,
};

use crate::NatsEnv;

const NODE_CONFIG_PATH: &str = "/etc/nats/nats.conf";

impl ComposeDeployEnv for NatsEnv {
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
        ComposeNodeConfigFileName::FixedExtension("nats").resolve(index)
    }

    fn loopback_node_runtime_spec(
        topology: &Self::Deployment,
        index: usize,
    ) -> Result<Option<LoopbackNodeRuntimeSpec>, DynError> {
        let _ = topology;
        Ok(Some(build_nats_runtime(index)))
    }
}

fn build_nats_runtime(index: usize) -> LoopbackNodeRuntimeSpec {
    let image = env::var("NATS_IMAGE").unwrap_or_else(|_| "nats:2.10".to_owned());
    let platform = env::var("NATS_PLATFORM").ok();

    LoopbackNodeRuntimeSpec {
        image,
        entrypoint: vec![
            "nats-server".to_owned(),
            "--config".to_owned(),
            NODE_CONFIG_PATH.to_owned(),
        ],
        volumes: vec![format!(
            "./stack/configs/{}.nats:{NODE_CONFIG_PATH}:ro",
            node_identifier(index)
        )],
        extra_hosts: vec![],
        container_ports: vec![8222, 4222, 6222],
        environment: vec![testing_framework_runner_compose::EnvEntry::new(
            "RUST_LOG", "info",
        )],
        platform,
    }
}
