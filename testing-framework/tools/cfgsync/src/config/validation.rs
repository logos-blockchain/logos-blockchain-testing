use testing_framework_config::topology::configs::consensus::ConsensusParams;
use thiserror::Error;

use crate::host::Host;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("host count {actual} does not match participants {expected}")]
    HostCountMismatch { actual: usize, expected: usize },
    #[error("id count {actual} does not match participants {expected}")]
    IdCountMismatch { actual: usize, expected: usize },
    #[error("da port count {actual} does not match participants {expected}")]
    DaPortCountMismatch { actual: usize, expected: usize },
    #[error("blend port count {actual} does not match participants {expected}")]
    BlendPortCountMismatch { actual: usize, expected: usize },
}

pub fn validate_inputs(
    hosts: &[Host],
    consensus_params: &ConsensusParams,
    ids: Option<&Vec<[u8; 32]>>,
    da_ports: Option<&Vec<u16>>,
    blend_ports: Option<&Vec<u16>>,
) -> Result<(), ValidationError> {
    let expected = consensus_params.n_participants;

    if hosts.len() != expected {
        return Err(ValidationError::HostCountMismatch {
            actual: hosts.len(),
            expected,
        });
    }

    if let Some(ids) = ids {
        if ids.len() != expected {
            return Err(ValidationError::IdCountMismatch {
                actual: ids.len(),
                expected,
            });
        }
    }

    if let Some(ports) = da_ports {
        if ports.len() != expected {
            return Err(ValidationError::DaPortCountMismatch {
                actual: ports.len(),
                expected,
            });
        }
    }

    if let Some(ports) = blend_ports {
        if ports.len() != expected {
            return Err(ValidationError::BlendPortCountMismatch {
                actual: ports.len(),
                expected,
            });
        }
    }

    Ok(())
}
