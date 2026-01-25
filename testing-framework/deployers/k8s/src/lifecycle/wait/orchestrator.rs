use kube::Client;
use testing_framework_core::scenario::http_probe::NodeRole;

use super::{ClusterPorts, ClusterReady, ClusterWaitError, NodeConfigPorts};
use crate::lifecycle::wait::{
    deployment::wait_for_deployment_ready,
    forwarding::{PortForwardHandle, kill_port_forwards, port_forward_group},
    http_probe::{wait_for_node_http_nodeport, wait_for_node_http_port_forward},
    ports::discover_node_ports,
};

pub async fn wait_for_cluster_ready(
    client: &Client,
    namespace: &str,
    release: &str,
    validator_ports: &[NodeConfigPorts],
) -> Result<ClusterReady, ClusterWaitError> {
    if validator_ports.is_empty() {
        return Err(ClusterWaitError::MissingValidator);
    }

    let mut validator_allocations = Vec::with_capacity(validator_ports.len());
    let mut validator_host = crate::host::node_host();

    for (index, ports) in validator_ports.iter().enumerate() {
        let name = format!("{release}-validator-{index}");
        wait_for_deployment_ready(client, namespace, &name).await?;
        let allocation = discover_node_ports(client, namespace, &name, *ports).await?;
        validator_allocations.push(allocation);
    }

    let mut port_forwards: Vec<PortForwardHandle> = Vec::new();

    let validator_api_ports: Vec<u16> = validator_allocations
        .iter()
        .map(|ports| ports.api)
        .collect();
    if wait_for_node_http_nodeport(&validator_api_ports, NodeRole::Validator)
        .await
        .is_err()
    {
        validator_allocations.clear();
        validator_host = "127.0.0.1".to_owned();
        let namespace = namespace.to_owned();
        let release = release.to_owned();
        let ports = validator_ports.to_vec();
        let (forwards, allocations) = tokio::task::spawn_blocking(move || {
            let mut allocations = Vec::with_capacity(ports.len());
            let forwards =
                port_forward_group(&namespace, &release, "validator", &ports, &mut allocations)?;
            Ok::<_, ClusterWaitError>((forwards, allocations))
        })
        .await
        .map_err(|source| ClusterWaitError::PortForwardTask {
            source: source.into(),
        })??;
        port_forwards = forwards;
        validator_allocations = allocations;
        let validator_api_ports: Vec<u16> = validator_allocations
            .iter()
            .map(|ports| ports.api)
            .collect();
        if let Err(err) =
            wait_for_node_http_port_forward(&validator_api_ports, NodeRole::Validator).await
        {
            kill_port_forwards(&mut port_forwards);
            return Err(err);
        }
    }

    Ok(ClusterReady {
        ports: ClusterPorts {
            validators: validator_allocations,
            validator_host,
        },
        port_forwards,
    })
}
