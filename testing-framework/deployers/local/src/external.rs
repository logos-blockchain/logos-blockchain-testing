use std::net::ToSocketAddrs;

use testing_framework_core::scenario::{DynError, ExternalNodeSource};

use crate::{LocalDeployerEnv, NodeEndpoints};

#[derive(Debug, thiserror::Error)]
pub enum ExternalClientBuildError {
    #[error("external source '{label}' endpoint is empty")]
    EmptyEndpoint { label: String },
    #[error("external source '{label}' endpoint '{endpoint}' has unsupported scheme")]
    UnsupportedScheme { label: String, endpoint: String },
    #[error("external source '{label}' endpoint '{endpoint}' is missing host")]
    MissingHost { label: String, endpoint: String },
    #[error("external source '{label}' endpoint '{endpoint}' is missing port")]
    MissingPort { label: String, endpoint: String },
    #[error("external source '{label}' endpoint '{endpoint}' failed to resolve: {source}")]
    Resolve {
        label: String,
        endpoint: String,
        #[source]
        source: std::io::Error,
    },
    #[error("external source '{label}' endpoint '{endpoint}' resolved to no socket addresses")]
    NoResolvedAddress { label: String, endpoint: String },
}

pub fn build_external_client<E: LocalDeployerEnv>(
    source: &ExternalNodeSource,
) -> Result<E::NodeClient, DynError> {
    let api = resolve_api_socket(source)?;
    let mut endpoints = NodeEndpoints::default();
    endpoints.api = api;
    Ok(E::node_client(&endpoints))
}

fn resolve_api_socket(source: &ExternalNodeSource) -> Result<std::net::SocketAddr, DynError> {
    let source_label = source.label().to_string();
    let endpoint = source.endpoint().trim();
    if endpoint.is_empty() {
        return Err(ExternalClientBuildError::EmptyEndpoint {
            label: source_label,
        }
        .into());
    }

    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .ok_or_else(|| ExternalClientBuildError::UnsupportedScheme {
            label: source_label.clone(),
            endpoint: endpoint.to_owned(),
        })?;

    let authority = without_scheme.trim_end_matches('/');
    let (host, port) =
        split_host_port(authority).ok_or_else(|| ExternalClientBuildError::MissingPort {
            label: source_label.clone(),
            endpoint: endpoint.to_owned(),
        })?;

    if host.is_empty() {
        return Err(ExternalClientBuildError::MissingHost {
            label: source_label.clone(),
            endpoint: endpoint.to_owned(),
        }
        .into());
    }

    let resolved = (host, port)
        .to_socket_addrs()
        .map_err(|source| ExternalClientBuildError::Resolve {
            label: source_label.clone(),
            endpoint: endpoint.to_owned(),
            source,
        })?
        .next()
        .ok_or_else(|| ExternalClientBuildError::NoResolvedAddress {
            label: source_label,
            endpoint: endpoint.to_owned(),
        })?;

    Ok(resolved)
}

fn split_host_port(authority: &str) -> Option<(&str, u16)> {
    let (host, port) = authority.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    Some((host.trim_matches(['[', ']']), port))
}
