use std::net::SocketAddr;

use nomos_utils::net::get_available_tcp_port;
use thiserror::Error;

const LOCALHOST: [u8; 4] = [127, 0, 0, 1];

#[derive(Clone)]
pub struct GeneralApiConfig {
    pub address: SocketAddr,
    pub testing_http_address: SocketAddr,
}

#[derive(Debug, Error)]
pub enum ApiConfigError {
    #[error("failed to allocate a free TCP port for API config")]
    PortAllocationFailed,
}

pub fn create_api_configs(ids: &[[u8; 32]]) -> Result<Vec<GeneralApiConfig>, ApiConfigError> {
    ids.iter()
        .map(|_| {
            let address_port =
                get_available_tcp_port().ok_or(ApiConfigError::PortAllocationFailed)?;
            let testing_port =
                get_available_tcp_port().ok_or(ApiConfigError::PortAllocationFailed)?;
            Ok(GeneralApiConfig {
                address: SocketAddr::from((LOCALHOST, address_port)),
                testing_http_address: SocketAddr::from((LOCALHOST, testing_port)),
            })
        })
        .collect()
}
