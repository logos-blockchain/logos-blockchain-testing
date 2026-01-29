use reqwest::Url;
use testing_framework_core::{nodes::ApiClient, scenario::http_probe};

use super::ManualClusterError;
use crate::{
    errors::{ComposeRunnerError, NodeClientError},
    infrastructure::ports::NodeHostPorts,
};

pub fn api_client_from_host_ports(
    ports: &NodeHostPorts,
    host: &str,
) -> Result<ApiClient, ManualClusterError> {
    let api_url = endpoint_url(host, "api", ports.api)?;
    let testing_url = endpoint_url(host, "testing", ports.testing)?;

    Ok(ApiClient::from_urls(api_url, Some(testing_url)))
}

fn endpoint_url(host: &str, endpoint: &'static str, port: u16) -> Result<Url, ManualClusterError> {
    Url::parse(&format!("http://{host}:{port}/")).map_err(|source| {
        ManualClusterError::Compose(ComposeRunnerError::NodeClients(NodeClientError::Endpoint {
            role: http_probe::NODE_ROLE,
            endpoint,
            port,
            source,
        }))
    })
}
