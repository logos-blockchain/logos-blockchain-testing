use kube::Client;
use testing_framework_core::scenario::http_probe::NodeRole;

use super::{
    ClusterPorts, ClusterReady, ClusterWaitError, NodeConfigPorts, PROMETHEUS_HTTP_PORT,
    PROMETHEUS_HTTP_PROBE_TIMEOUT, PROMETHEUS_SERVICE_NAME,
};
use crate::lifecycle::wait::{
    deployment::wait_for_deployment_ready,
    forwarding::{kill_port_forwards, port_forward_group, port_forward_service},
    http_probe::{wait_for_node_http_nodeport, wait_for_node_http_port_forward},
    ports::{discover_node_ports, find_node_port},
    prometheus::{wait_for_prometheus_http_nodeport, wait_for_prometheus_http_port_forward},
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

    for (index, ports) in validator_ports.iter().enumerate() {
        let name = format!("{release}-validator-{index}");
        wait_for_deployment_ready(client, namespace, &name).await?;
        let allocation = discover_node_ports(client, namespace, &name, *ports).await?;
        validator_allocations.push(allocation);
    }

    let mut port_forwards = Vec::new();

    let validator_api_ports: Vec<u16> = validator_allocations
        .iter()
        .map(|ports| ports.api)
        .collect();
    if wait_for_node_http_nodeport(&validator_api_ports, NodeRole::Validator)
        .await
        .is_err()
    {
        validator_allocations.clear();
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
        match port_forward_group(
            namespace,
            release,
            "executor",
            executor_ports,
            &mut executor_allocations,
        ) {
            Ok(forwards) => port_forwards.extend(forwards),
            Err(err) => {
                kill_port_forwards(&mut port_forwards);
                return Err(err);
            }
        }
        let executor_api_ports: Vec<u16> =
            executor_allocations.iter().map(|ports| ports.api).collect();
        if let Err(err) =
            wait_for_node_http_port_forward(&executor_api_ports, NodeRole::Executor).await
        {
            kill_port_forwards(&mut port_forwards);
            return Err(err);
        }
    }

    let mut prometheus_port = find_node_port(
        client,
        namespace,
        PROMETHEUS_SERVICE_NAME,
        PROMETHEUS_HTTP_PORT,
    )
    .await?;
    if wait_for_prometheus_http_nodeport(prometheus_port, PROMETHEUS_HTTP_PROBE_TIMEOUT)
        .await
        .is_err()
    {
        let (local_port, forward) =
            port_forward_service(namespace, PROMETHEUS_SERVICE_NAME, PROMETHEUS_HTTP_PORT)
                .map_err(|err| {
                    kill_port_forwards(&mut port_forwards);
                    err
                })?;
        prometheus_port = local_port;
        port_forwards.push(forward);
        if let Err(err) = wait_for_prometheus_http_port_forward(prometheus_port).await {
            kill_port_forwards(&mut port_forwards);
            return Err(err);
        }
    }

    Ok(ClusterReady {
        ports: ClusterPorts {
            validators: validator_allocations,
            executors: executor_allocations,
            prometheus: prometheus_port,
        },
        port_forwards,
    })
}
