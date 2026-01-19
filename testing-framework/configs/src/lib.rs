use std::{net::Ipv4Addr, ops::Mul as _, sync::LazyLock, time::Duration};

use nomos_core::sdp::ProviderId;
use nomos_libp2p::{Multiaddr, PeerId, multiaddr};
use testing_framework_env as tf_env;

pub mod constants;
pub mod nodes;
pub mod timeouts;
pub mod topology;

static IS_SLOW_TEST_ENV: LazyLock<bool> = LazyLock::new(tf_env::slow_test_env);

pub static IS_DEBUG_TRACING: LazyLock<bool> = LazyLock::new(tf_env::debug_tracing);

const SLOW_ENV_TIMEOUT_MULTIPLIER: u32 = 2;

/// In slow test environments like Codecov, use 2x timeout.
#[must_use]
pub fn adjust_timeout(d: Duration) -> Duration {
    if *IS_SLOW_TEST_ENV {
        d.mul(SLOW_ENV_TIMEOUT_MULTIPLIER)
    } else {
        d
    }
}

#[must_use]
pub fn node_address_from_port(port: u16) -> Multiaddr {
    multiaddr(Ipv4Addr::LOCALHOST, port)
}

#[must_use]
pub fn secret_key_to_peer_id(node_key: nomos_libp2p::ed25519::SecretKey) -> PeerId {
    PeerId::from_public_key(
        &nomos_libp2p::ed25519::Keypair::from(node_key)
            .public()
            .into(),
    )
}

#[must_use]
pub fn secret_key_to_provider_id(node_key: nomos_libp2p::ed25519::SecretKey) -> ProviderId {
    let bytes = nomos_libp2p::ed25519::Keypair::from(node_key)
        .public()
        .to_bytes();
    match ProviderId::try_from(bytes) {
        Ok(value) => value,
        Err(_) => unsafe {
            // Safety: `bytes` is a 32-byte ed25519 public key, matching `ProviderId`'s
            // expected width; failure would indicate a broken invariant in the
            // dependency.
            std::hint::unreachable_unchecked()
        },
    }
}
