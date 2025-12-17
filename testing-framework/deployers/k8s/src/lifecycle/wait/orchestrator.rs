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
    executor_ports: &[NodeConfigPorts],
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
        port_forwards = port_forward_group(
            namespace,
            release,
            "validator",
            validator_ports,
            &mut validator_allocations,
        )?;
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

    let mut executor_allocations = Vec::with_capacity(executor_ports.len());
    let mut executor_host = crate::host::node_host();
    for (index, ports) in executor_ports.iter().enumerate() {
        let name = format!("{release}-executor-{index}");
        wait_for_deployment_ready(client, namespace, &name).await?;
        let allocation = discover_node_ports(client, namespace, &name, *ports).await?;
        executor_allocations.push(allocation);
    }

    let executor_api_ports: Vec<u16> = executor_allocations.iter().map(|ports| ports.api).collect();
    if !executor_allocations.is_empty()
        && wait_for_node_http_nodeport(&executor_api_ports, NodeRole::Executor)
            .await
            .is_err()
    {
        executor_allocations.clear();
        executor_host = "127.0.0.1".to_owned();
        match port_forward_group(
            namespace,
            release,
            "executor",
            executor_ports,
            &mut executor_allocations,
        ) {
            Ok(forwards) => port_forwards.extend(forwards),
            Err(err) => return Err(cleanup_port_forwards(&mut port_forwards, err)),
        }
        let executor_api_ports: Vec<u16> =
            executor_allocations.iter().map(|ports| ports.api).collect();
        if let Err(err) =
            wait_for_node_http_port_forward(&executor_api_ports, NodeRole::Executor).await
        {
            return Err(cleanup_port_forwards(&mut port_forwards, err));
        }
    }

    Ok(ClusterReady {
        ports: ClusterPorts {
            validators: validator_allocations,
            executors: executor_allocations,
            validator_host,
            executor_host,
        },
        port_forwards,
    })
}

fn cleanup_port_forwards(
    port_forwards: &mut Vec<PortForwardHandle>,
    error: ClusterWaitError,
) -> ClusterWaitError {
    kill_port_forwards(port_forwards);
    error
}
