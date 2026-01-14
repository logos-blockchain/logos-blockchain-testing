use nomos_libp2p::{Multiaddr, PeerId, Protocol};
use thiserror::Error;

use super::address::find_matching_host;
use crate::host::Host;

#[derive(Debug, Error)]
pub enum PeerRewriteError {
    #[error("hosts and peer ids length mismatch (hosts={hosts}, peer_ids={peer_ids})")]
    HostPeerLenMismatch { hosts: usize, peer_ids: usize },
    #[error("peer index {peer_idx} out of bounds for hosts (len={hosts_len})")]
    HostIndexOutOfBounds { peer_idx: usize, hosts_len: usize },
    #[error("peer index {peer_idx} out of bounds for peer ids (len={peer_ids_len})")]
    PeerIdIndexOutOfBounds {
        peer_idx: usize,
        peer_ids_len: usize,
    },
}

pub fn rewrite_initial_peers(
    templates: &[Vec<Multiaddr>],
    original_ports: &[u16],
    hosts: &[Host],
    peer_ids: &[PeerId],
) -> Result<Vec<Vec<Multiaddr>>, PeerRewriteError> {
    if hosts.len() != peer_ids.len() {
        return Err(PeerRewriteError::HostPeerLenMismatch {
            hosts: hosts.len(),
            peer_ids: peer_ids.len(),
        });
    }

    let mut rewritten = Vec::with_capacity(templates.len());
    for (node_idx, peers) in templates.iter().enumerate() {
        let mut node_peers = Vec::new();
        for addr in peers {
            let Some(peer_idx) = find_matching_host(addr, original_ports) else {
                continue;
            };
            if peer_idx == node_idx {
                continue;
            }

            let host = hosts
                .get(peer_idx)
                .ok_or(PeerRewriteError::HostIndexOutOfBounds {
                    peer_idx,
                    hosts_len: hosts.len(),
                })?;
            let peer_id =
                peer_ids
                    .get(peer_idx)
                    .ok_or(PeerRewriteError::PeerIdIndexOutOfBounds {
                        peer_idx,
                        peer_ids_len: peer_ids.len(),
                    })?;

            let mut rewritten_addr = Multiaddr::empty();
            rewritten_addr.push(Protocol::Ip4(host.ip));
            rewritten_addr.push(Protocol::Udp(host.network_port));
            rewritten_addr.push(Protocol::QuicV1);
            rewritten_addr.push(Protocol::P2p((*peer_id).into()));
            node_peers.push(rewritten_addr);
        }
        rewritten.push(node_peers);
    }

    Ok(rewritten)
}
