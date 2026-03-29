use std::time::Duration;

use k8s_openapi::api::core::v1::{Service, ServicePort};
use kube::{Api, Client};
use tokio_retry::{RetryIf, strategy::FixedInterval};

use super::{ClusterWaitError, NodeConfigPorts, NodePortAllocation};

const NODE_PORT_LOOKUP_ATTEMPTS: u32 = 120;
const NODE_PORT_LOOKUP_ATTEMPTS_USIZE: usize = NODE_PORT_LOOKUP_ATTEMPTS as usize;
const NODE_PORT_LOOKUP_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
enum NodePortLookupError {
    NotAvailable,
    Fatal(ClusterWaitError),
}

pub async fn find_node_port(
    client: &Client,
    namespace: &str,
    service_name: &str,
    service_port: u16,
) -> Result<u16, ClusterWaitError> {
    let services = Api::<Service>::namespaced(client.clone(), namespace);
    let strategy = port_lookup_retry_strategy();
    let result = RetryIf::spawn(
        strategy,
        || query_node_port(&services, service_name, service_port),
        |error: &NodePortLookupError| matches!(error, NodePortLookupError::NotAvailable),
    )
    .await;

    map_node_port_lookup_result(result, service_name, service_port)
}

pub async fn discover_node_ports(
    client: &Client,
    namespace: &str,
    service_name: &str,
    config_ports: NodeConfigPorts,
) -> Result<NodePortAllocation, ClusterWaitError> {
    let api_port = find_node_port(client, namespace, service_name, config_ports.api).await?;
    let auxiliary_port =
        find_node_port(client, namespace, service_name, config_ports.auxiliary).await?;

    Ok(NodePortAllocation {
        api: api_port,
        auxiliary: auxiliary_port,
    })
}

fn port_lookup_retry_strategy() -> impl Iterator<Item = Duration> {
    FixedInterval::from_millis(NODE_PORT_LOOKUP_INTERVAL.as_millis() as u64)
        .take(NODE_PORT_LOOKUP_ATTEMPTS_USIZE)
}

async fn query_node_port(
    services: &Api<Service>,
    service_name: &str,
    service_port: u16,
) -> Result<u16, NodePortLookupError> {
    match services.get(service_name).await {
        Ok(service) => lookup_service_node_port(service, service_port),
        Err(source) => Err(NodePortLookupError::Fatal(ClusterWaitError::ServiceFetch {
            service: service_name.to_owned(),
            source,
        })),
    }
}

fn lookup_service_node_port(
    service: Service,
    service_port: u16,
) -> Result<u16, NodePortLookupError> {
    let ports = service.spec.and_then(|spec| spec.ports).unwrap_or_default();

    for port in ports {
        if let Some(node_port) = matching_node_port(&port, service_port) {
            return Ok(node_port as u16);
        }
    }

    Err(NodePortLookupError::NotAvailable)
}

fn matching_node_port(port: &ServicePort, service_port: u16) -> Option<i32> {
    if port.port != i32::from(service_port) {
        return None;
    }

    port.node_port
}

fn map_node_port_lookup_result(
    result: Result<u16, NodePortLookupError>,
    service_name: &str,
    service_port: u16,
) -> Result<u16, ClusterWaitError> {
    match result {
        Ok(port) => Ok(port),
        Err(NodePortLookupError::Fatal(error)) => Err(error),
        Err(NodePortLookupError::NotAvailable) => Err(ClusterWaitError::NodePortUnavailable {
            service: service_name.to_owned(),
            port: service_port,
        }),
    }
}
