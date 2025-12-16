use kube::Client;
use testing_framework_core::scenario::http_probe::NodeRole;

use super::{
    ClusterPorts, ClusterReady, ClusterWaitError, HostPort, NodeConfigPorts, PROMETHEUS_HTTP_PORT,
    PROMETHEUS_SERVICE_NAME, prometheus_http_probe_timeout,
};
use crate::lifecycle::wait::{
    deployment::wait_for_deployment_ready,
    forwarding::{
        PortForwardHandle, PortForwardSpawn, kill_port_forwards, port_forward_group,
        port_forward_service,
    },
    grafana::{wait_for_grafana_http_nodeport, wait_for_grafana_http_port_forward},
    http_probe::{wait_for_node_http_nodeport, wait_for_node_http_port_forward},
    ports::{discover_node_ports, find_node_port},
    prometheus::{wait_for_prometheus_http_nodeport, wait_for_prometheus_http_port_forward},
};

const GRAFANA_HTTP_PORT: u16 = 3000;

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

    let mut prometheus_port = find_node_port(
        client,
        namespace,
        PROMETHEUS_SERVICE_NAME,
        PROMETHEUS_HTTP_PORT,
    )
    .await?;
    let mut prometheus_host = crate::host::node_host();
    if wait_for_prometheus_http_nodeport(prometheus_port, prometheus_http_probe_timeout())
        .await
        .is_err()
    {
        let PortForwardSpawn { local_port, handle } =
            port_forward_service(namespace, PROMETHEUS_SERVICE_NAME, PROMETHEUS_HTTP_PORT)
                .map_err(|err| {
                    kill_port_forwards(&mut port_forwards);
                    err
                })?;
        prometheus_port = local_port;
        prometheus_host = "127.0.0.1".to_owned();
        port_forwards.push(handle);
        if let Err(err) = wait_for_prometheus_http_port_forward(prometheus_port).await {
            return Err(cleanup_port_forwards(&mut port_forwards, err));
        }
    }

    let mut grafana = None;
    let grafana_service = format!("{release}-grafana");
    if let Ok(node_port) =
        find_node_port(client, namespace, &grafana_service, GRAFANA_HTTP_PORT).await
    {
        let mut grafana_host = crate::host::node_host();
        let mut grafana_port = node_port;
        if wait_for_grafana_http_nodeport(grafana_port).await.is_err() {
            let PortForwardSpawn { local_port, handle } =
                port_forward_service(namespace, &grafana_service, GRAFANA_HTTP_PORT).map_err(
                    |err| {
                        kill_port_forwards(&mut port_forwards);
                        err
                    },
                )?;
            grafana_host = "127.0.0.1".to_owned();
            grafana_port = local_port;
            port_forwards.push(handle);
            if let Err(err) = wait_for_grafana_http_port_forward(grafana_port).await {
                return Err(cleanup_port_forwards(&mut port_forwards, err));
            }
        }
        grafana = Some(HostPort {
            host: grafana_host,
            port: grafana_port,
        });
    }

    Ok(ClusterReady {
        ports: ClusterPorts {
            validators: validator_allocations,
            executors: executor_allocations,
            validator_host,
            executor_host,
            prometheus: HostPort {
                host: prometheus_host,
                port: prometheus_port,
            },
            grafana,
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
