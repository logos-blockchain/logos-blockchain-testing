use kube::Client;

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
    node_ports: &[NodeConfigPorts],
) -> Result<ClusterReady, ClusterWaitError> {
    if node_ports.is_empty() {
        return Err(ClusterWaitError::MissingNode);
    }

    let mut node_allocations = Vec::with_capacity(node_ports.len());
    let mut node_host = crate::host::node_host();

    for (index, ports) in node_ports.iter().enumerate() {
        let name = format!("{release}-node-{index}");
        wait_for_deployment_ready(client, namespace, &name).await?;
        let allocation = discover_node_ports(client, namespace, &name, *ports).await?;
        node_allocations.push(allocation);
    }

    let mut port_forwards: Vec<PortForwardHandle> = Vec::new();

    let node_api_ports: Vec<u16> = node_allocations.iter().map(|ports| ports.api).collect();
    if wait_for_node_http_nodeport(&node_api_ports, "node")
        .await
        .is_err()
    {
        node_allocations.clear();
        node_host = "127.0.0.1".to_owned();
        let namespace = namespace.to_owned();
        let release = release.to_owned();
        let ports = node_ports.to_vec();
        let (forwards, allocations) = tokio::task::spawn_blocking(move || {
            let mut allocations = Vec::with_capacity(ports.len());
            let forwards =
                port_forward_group(&namespace, &release, "node", &ports, &mut allocations)?;
            Ok::<_, ClusterWaitError>((forwards, allocations))
        })
        .await
        .map_err(|source| ClusterWaitError::PortForwardTask {
            source: source.into(),
        })??;
        port_forwards = forwards;
        node_allocations = allocations;
        let node_api_ports: Vec<u16> = node_allocations.iter().map(|ports| ports.api).collect();
        if let Err(err) = wait_for_node_http_port_forward(&node_api_ports, "node").await {
            kill_port_forwards(&mut port_forwards);
            return Err(err);
        }
    }

    Ok(ClusterReady {
        ports: ClusterPorts {
            nodes: node_allocations,
            node_host,
        },
        port_forwards,
    })
}
