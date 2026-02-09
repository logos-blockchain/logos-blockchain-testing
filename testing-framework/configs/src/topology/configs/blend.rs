use core::time::Duration;
use std::{net::Ipv4Addr, num::NonZeroU64};

use lb_blend_service::{
    core::backends::libp2p::Libp2pBlendBackendSettings as Libp2pCoreBlendBackendSettings,
    edge::backends::libp2p::Libp2pBlendBackendSettings as Libp2pEdgeBlendBackendSettings,
};
use lb_key_management_system_service::keys::{Ed25519Key, UnsecuredEd25519Key, ZkKey};
use lb_libp2p::{Multiaddr, Protocol, protocol_name::StreamProtocol};
use lb_utils::math::NonNegativeF64;
use num_bigint::BigUint;

const EDGE_NODE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(1);
const LOCALHOST: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);

#[derive(Clone)]
pub struct GeneralBlendConfig {
    pub backend_core: Libp2pCoreBlendBackendSettings,
    pub backend_edge: Libp2pEdgeBlendBackendSettings,
    pub private_key: UnsecuredEd25519Key,
    pub secret_zk_key: ZkKey,
    pub signer: Ed25519Key,
}

/// Builds blend configs for each node.
#[must_use]
pub fn create_blend_configs(ids: &[[u8; 32]], ports: &[u16]) -> Vec<GeneralBlendConfig> {
    ids.iter()
        .zip(ports)
        .map(|(id, port)| {
            let signer = Ed25519Key::from_bytes(id);
            let private_key = UnsecuredEd25519Key::from_bytes(id);
            // We need unique ZK secret keys, so we just derive them deterministically from
            // the generated Ed25519 public keys, which are guaranteed to be unique because
            // they are in turned derived from node ID.
            let secret_zk_key =
                ZkKey::from(BigUint::from_bytes_le(private_key.public_key().as_bytes()));
            let listening_address = localhost_quic_address(*port);
            let minimum_messages_coefficient = unsafe { NonZeroU64::new_unchecked(1) };
            let normalization_constant = match NonNegativeF64::try_from(1.03f64) {
                Ok(value) => value,
                Err(_) => unsafe {
                    // Safety: normalization constant is a finite non-negative constant.
                    std::hint::unreachable_unchecked()
                },
            };
            let max_dial_attempts_per_peer = unsafe { NonZeroU64::new_unchecked(3) };
            let max_dial_attempts_per_peer_per_message = match 1.try_into() {
                Ok(value) => value,
                Err(_) => unsafe {
                    // Safety: the constant 1 must fit the target type and be non-zero.
                    std::hint::unreachable_unchecked()
                },
            };
            let replication_factor = match 1.try_into() {
                Ok(value) => value,
                Err(_) => unsafe {
                    // Safety: the constant 1 must fit the target type and be non-zero.
                    std::hint::unreachable_unchecked()
                },
            };
            GeneralBlendConfig {
                backend_core: Libp2pCoreBlendBackendSettings {
                    listening_address,
                    core_peering_degree: 1..=3,
                    minimum_messages_coefficient,
                    normalization_constant,
                    edge_node_connection_timeout: EDGE_NODE_CONNECTION_TIMEOUT,
                    max_edge_node_incoming_connections: 300,
                    max_dial_attempts_per_peer,
                    protocol_name: StreamProtocol::new("/blend/integration-tests"),
                },
                backend_edge: Libp2pEdgeBlendBackendSettings {
                    max_dial_attempts_per_peer_per_message,
                    protocol_name: StreamProtocol::new("/blend/integration-tests"),
                    replication_factor,
                },
                private_key,
                secret_zk_key,
                signer,
            }
        })
        .collect()
}

fn localhost_quic_address(port: u16) -> Multiaddr {
    let mut addr = Multiaddr::empty();
    addr.push(Protocol::Ip4(LOCALHOST));
    addr.push(Protocol::Udp(port));
    addr.push(Protocol::QuicV1);
    addr
}
