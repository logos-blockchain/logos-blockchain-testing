use testing_framework_config::topology::{
    configs::consensus::ConsensusParams,
    invariants::{TopologyInvariantError, validate_node_vectors},
};
use thiserror::Error;

use crate::host::Host;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("host count {actual} does not match participants {expected}")]
    HostCountMismatch { actual: usize, expected: usize },
    #[error(transparent)]
    TopologyInvariant(#[from] TopologyInvariantError),
}

pub fn validate_inputs(
    hosts: &[Host],
    consensus_params: &ConsensusParams,
    ids: Option<&Vec<[u8; 32]>>,
    blend_ports: Option<&Vec<u16>>,
) -> Result<(), ValidationError> {
    let expected = consensus_params.n_participants;

    if hosts.len() != expected {
        return Err(ValidationError::HostCountMismatch {
            actual: hosts.len(),
            expected,
        });
    }

    validate_node_vectors(expected, ids, blend_ports)?;

    Ok(())
}
