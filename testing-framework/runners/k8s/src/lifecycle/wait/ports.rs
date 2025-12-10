use k8s_openapi::api::core::v1::Service;
use kube::{Api, Client};
use tokio::time::sleep;

use super::{ClusterWaitError, NodeConfigPorts, NodePortAllocation};

pub async fn find_node_port(
    client: &Client,
    namespace: &str,
    service_name: &str,
    service_port: u16,
) -> Result<u16, ClusterWaitError> {
    let interval = std::time::Duration::from_secs(1);
    for _ in 0..120 {
        match Api::<Service>::namespaced(client.clone(), namespace)
            .get(service_name)
            .await
        {
            Ok(service) => {
                if let Some(spec) = service.spec.clone()
                    && let Some(ports) = spec.ports
                {
                    for port in ports {
                        if port.port == i32::from(service_port)
                            && let Some(node_port) = port.node_port
                        {
                            return Ok(node_port as u16);
                        }
                    }
                }
            }
            Err(err) => {
                return Err(ClusterWaitError::ServiceFetch {
                    service: service_name.to_owned(),
                    source: err,
                });
            }
        }
        sleep(interval).await;
    }

    Err(ClusterWaitError::NodePortUnavailable {
        service: service_name.to_owned(),
        port: service_port,
    })
}

pub async fn discover_node_ports(
    client: &Client,
    namespace: &str,
    service_name: &str,
    config_ports: NodeConfigPorts,
) -> Result<NodePortAllocation, ClusterWaitError> {
    let api_port = find_node_port(client, namespace, service_name, config_ports.api).await?;
    let testing_port =
        find_node_port(client, namespace, service_name, config_ports.testing).await?;

    Ok(NodePortAllocation {
        api: api_port,
        testing: testing_port,
    })
}
