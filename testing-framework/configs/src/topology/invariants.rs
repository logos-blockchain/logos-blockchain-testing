use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TopologyInvariantError {
    #[error("participant count must be > 0")]
    EmptyParticipants,
    #[error("id count {actual} does not match participants {expected}")]
    IdCountMismatch { actual: usize, expected: usize },
    #[error("da port count {actual} does not match participants {expected}")]
    DaPortCountMismatch { actual: usize, expected: usize },
    #[error("blend port count {actual} does not match participants {expected}")]
    BlendPortCountMismatch { actual: usize, expected: usize },
}

/// Validate basic invariants shared across all config generation pipelines.
///
/// This intentionally focuses on "shape" invariants (counts, non-empty) and
/// avoids opinionated checks so behavior stays unchanged.
pub fn validate_node_vectors(
    participants: usize,
    ids: Option<&Vec<[u8; 32]>>,
    da_ports: Option<&Vec<u16>>,
    blend_ports: Option<&Vec<u16>>,
) -> Result<(), TopologyInvariantError> {
    if participants == 0 {
        return Err(TopologyInvariantError::EmptyParticipants);
    }

    if let Some(ids) = ids {
        if ids.len() != participants {
            return Err(TopologyInvariantError::IdCountMismatch {
                actual: ids.len(),
                expected: participants,
            });
        }
    }

    if let Some(ports) = da_ports {
        if ports.len() != participants {
            return Err(TopologyInvariantError::DaPortCountMismatch {
                actual: ports.len(),
                expected: participants,
            });
        }
    }

    if let Some(ports) = blend_ports {
        if ports.len() != participants {
            return Err(TopologyInvariantError::BlendPortCountMismatch {
                actual: ports.len(),
                expected: participants,
            });
        }
    }

    Ok(())
}

pub fn validate_generated_vectors(
    participants: usize,
    ids: &[[u8; 32]],
    da_ports: &[u16],
    blend_ports: &[u16],
) -> Result<(), TopologyInvariantError> {
    if participants == 0 {
        return Err(TopologyInvariantError::EmptyParticipants);
    }

    if ids.len() != participants {
        return Err(TopologyInvariantError::IdCountMismatch {
            actual: ids.len(),
            expected: participants,
        });
    }

    if da_ports.len() != participants {
        return Err(TopologyInvariantError::DaPortCountMismatch {
            actual: da_ports.len(),
            expected: participants,
        });
    }

    if blend_ports.len() != participants {
        return Err(TopologyInvariantError::BlendPortCountMismatch {
            actual: blend_ports.len(),
            expected: participants,
        });
    }

    Ok(())
}
