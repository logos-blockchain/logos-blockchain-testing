use kube::Client;
use tracing::warn;

use super::{ClusterPorts, ClusterReady, ClusterWaitError, NodeConfigPorts, NodePortAllocation};
use crate::{
    env::{K8sDeployEnv, node_deployment_name, node_role, node_service_name},
    lifecycle::wait::{
        ForwardSpec,
        deployment::wait_for_deployment_ready,
        forwarding::{PortForwardRegistry, PortForwardSpawn, port_forward_service},
        http_probe::{wait_for_node_http_nodeport, wait_for_node_http_port_forward},
        ports::discover_node_ports,
    },
};

const LOCALHOST: &str = "127.0.0.1";

pub async fn wait_for_cluster_ready<E: K8sDeployEnv>(
    client: &Client,
    namespace: &str,
    release: &str,
    node_ports: &[NodeConfigPorts],
) -> Result<ClusterReady, ClusterWaitError> {
    if node_ports.is_empty() {
        return Err(ClusterWaitError::MissingNode);
    }

    let node_allocations =
        discover_ready_node_allocations::<E>(client, namespace, release, node_ports).await?;
    let role = node_role::<E>();

    let (readiness, node_allocations) =
        if needs_port_forward_fallback::<E>(&node_allocations, role).await {
            fallback_readiness::<E>(namespace, release, node_ports, role).await?
        } else {
            (direct_nodeport_readiness(), node_allocations)
        };

    Ok(cluster_ready(readiness, node_allocations))
}

struct ReadinessResolution {
    node_host: String,
    port_forwards: PortForwardRegistry,
}

async fn needs_port_forward_fallback<E: K8sDeployEnv>(
    allocations: &[NodePortAllocation],
    role: &'static str,
) -> bool {
    let ports = api_ports(allocations);

    match wait_for_node_http_nodeport::<E>(&ports, role).await {
        Ok(()) => false,
        Err(error) => {
            warn!(
                %error,
                "NodePort readiness probe failed; falling back to kubectl port-forward"
            );
            true
        }
    }
}

async fn resolve_with_port_forwards<E: K8sDeployEnv>(
    namespace: &str,
    release: &str,
    node_ports: &[NodeConfigPorts],
    role: &'static str,
) -> Result<(PortForwardRegistry, Vec<NodePortAllocation>), ClusterWaitError> {
    let (port_forwards, node_allocations) = spawn_port_forwards::<E>(
        namespace.to_owned(),
        release.to_owned(),
        node_ports.to_vec(),
    )
    .await?;

    if let Err(error) =
        wait_for_node_http_port_forward::<E>(&api_ports(&node_allocations), role).await
    {
        port_forwards.shutdown_all_async().await;
        return Err(error);
    }

    Ok((port_forwards, node_allocations))
}

async fn fallback_readiness<E: K8sDeployEnv>(
    namespace: &str,
    release: &str,
    node_ports: &[NodeConfigPorts],
    role: &'static str,
) -> Result<(ReadinessResolution, Vec<NodePortAllocation>), ClusterWaitError> {
    let (port_forwards, allocations) =
        resolve_with_port_forwards::<E>(namespace, release, node_ports, role).await?;

    Ok((
        ReadinessResolution {
            node_host: LOCALHOST.to_owned(),
            port_forwards,
        },
        allocations,
    ))
}

fn direct_nodeport_readiness() -> ReadinessResolution {
    ReadinessResolution {
        node_host: crate::host::node_host(),
        port_forwards: PortForwardRegistry::default(),
    }
}

async fn discover_ready_node_allocations<E: K8sDeployEnv>(
    client: &Client,
    namespace: &str,
    release: &str,
    node_ports: &[NodeConfigPorts],
) -> Result<Vec<NodePortAllocation>, ClusterWaitError> {
    let mut allocations = Vec::with_capacity(node_ports.len());

    for (index, ports) in node_ports.iter().enumerate() {
        let deployment_name = node_deployment_name::<E>(release, index);
        wait_for_deployment_ready(client, namespace, &deployment_name).await?;
        let allocation = discover_node_ports(client, namespace, &deployment_name, *ports).await?;
        allocations.push(allocation);
    }

    Ok(allocations)
}

async fn spawn_port_forwards<E: K8sDeployEnv>(
    namespace: String,
    release: String,
    node_ports: Vec<NodeConfigPorts>,
) -> Result<(PortForwardRegistry, Vec<NodePortAllocation>), ClusterWaitError> {
    tokio::task::spawn_blocking(move || {
        let mut allocations = Vec::with_capacity(node_ports.len());
        let forwards = PortForwardRegistry::default();

        for (index, ports) in node_ports.iter().enumerate() {
            let service = node_service_name::<E>(&release, index);
            let api_forward = port_forward_service(&namespace, &service, ports.api)?;
            let auxiliary_forward = port_forward_service(&namespace, &service, ports.auxiliary)?;
            register_forward_pair(
                &namespace,
                &service,
                *ports,
                &mut allocations,
                &forwards,
                api_forward,
                auxiliary_forward,
            );
        }

        Ok::<_, ClusterWaitError>((forwards, allocations))
    })
    .await
    .map_err(|source| ClusterWaitError::PortForwardTask {
        source: source.into(),
    })?
}

fn api_ports(nodes: &[NodePortAllocation]) -> Vec<u16> {
    nodes.iter().map(|ports| ports.api).collect()
}

fn cluster_ready(
    readiness: ReadinessResolution,
    node_allocations: Vec<NodePortAllocation>,
) -> ClusterReady {
    ClusterReady {
        ports: ClusterPorts {
            nodes: node_allocations,
            node_host: readiness.node_host,
        },
        port_forwards: readiness.port_forwards,
    }
}

fn register_forward_pair(
    namespace: &str,
    service: &str,
    ports: NodeConfigPorts,
    allocations: &mut Vec<NodePortAllocation>,
    forwards: &PortForwardRegistry,
    api_forward: PortForwardSpawn,
    auxiliary_forward: PortForwardSpawn,
) {
    allocations.push(NodePortAllocation {
        api: api_forward.local_port,
        auxiliary: auxiliary_forward.local_port,
    });
    forwards.register_node(
        ForwardSpec {
            namespace: namespace.to_owned(),
            service: service.to_owned(),
            local_port: api_forward.local_port,
            remote_port: ports.api,
        },
        api_forward.handle,
        ForwardSpec {
            namespace: namespace.to_owned(),
            service: service.to_owned(),
            local_port: auxiliary_forward.local_port,
            remote_port: ports.auxiliary,
        },
        auxiliary_forward.handle,
    );
}
