use std::{fs, path::Path};

use testing_framework_core::{
    cfgsync::StaticNodeConfigProvider,
    scenario::{Application, DynError},
    topology::DeploymentDescriptor,
};
use testing_framework_runner_compose::{
    BinaryConfigNodeSpec, ComposeDeployEnv, ComposeDescriptor, NodeDescriptor,
    binary_config_node_runtime_spec, node_identifier,
};

use crate::OpenRaftKvEnv;

const NODE_CONFIG_PATH: &str = "/etc/openraft-kv/config.yaml";
const COMPOSE_HTTP_PORT_BASE: u16 = 47_080;

fn compose_node_spec() -> BinaryConfigNodeSpec {
    BinaryConfigNodeSpec::conventional(
        "/usr/local/bin/openraft-kv-node",
        NODE_CONFIG_PATH,
        vec![8080],
    )
}

fn fixed_loopback_port_binding(host_port: u16, container_port: u16) -> String {
    format!("127.0.0.1:{host_port}:{container_port}")
}

impl ComposeDeployEnv for OpenRaftKvEnv {
    fn prepare_compose_configs(
        path: &Path,
        topology: &<Self as Application>::Deployment,
        _cfgsync_port: u16,
        _metrics_otlp_ingest_url: Option<&reqwest::Url>,
    ) -> Result<(), DynError> {
        let hostnames = Self::cfgsync_hostnames(topology);
        let stack_dir = path
            .parent()
            .ok_or_else(|| std::io::Error::other("compose config path has no parent"))?;
        let configs_dir = stack_dir.join("configs");
        fs::create_dir_all(&configs_dir)?;

        for index in 0..topology.node_count() {
            let mut config = Self::build_node_config(topology, index)?;
            Self::rewrite_for_hostnames(topology, index, &hostnames, &mut config)?;
            let rendered = Self::serialize_node_config(&config)?;
            fs::write(
                configs_dir.join(Self::static_node_config_file_name(index)),
                rendered,
            )?;
        }

        Ok(())
    }

    fn static_node_config_file_name(index: usize) -> String {
        format!("node-{index}.yaml")
    }

    fn binary_config_node_spec(
        _topology: &<Self as Application>::Deployment,
        _index: usize,
    ) -> Result<Option<BinaryConfigNodeSpec>, DynError> {
        Ok(Some(compose_node_spec()))
    }

    fn compose_descriptor(
        topology: &<Self as Application>::Deployment,
        _cfgsync_port: u16,
    ) -> Result<ComposeDescriptor, DynError> {
        let spec = compose_node_spec();

        let nodes = (0..topology.node_count())
            .map(|index| {
                let runtime = binary_config_node_runtime_spec(index, &spec);
                let file_name = Self::static_node_config_file_name(index);

                let host_port = COMPOSE_HTTP_PORT_BASE + index as u16;
                let ports = compose_node_ports(host_port, &runtime.container_ports);

                NodeDescriptor::new(
                    node_identifier(index),
                    runtime.image,
                    runtime.entrypoint,
                    vec![format!(
                        "./stack/configs/{file_name}:{}:ro",
                        spec.config_container_path
                    )],
                    runtime.extra_hosts,
                    ports,
                    runtime.container_ports,
                    runtime.environment,
                    runtime.platform,
                )
            })
            .collect();

        Ok(ComposeDescriptor::new(nodes))
    }
}

fn compose_node_ports(host_port: u16, container_ports: &[u16]) -> Vec<String> {
    container_ports
        .iter()
        .map(|port| {
            // OpenRaft failover restarts the leader. Fixed host ports keep TF
            // clients stable across `docker compose restart`.
            fixed_loopback_port_binding(host_port, *port)
        })
        .collect()
}
