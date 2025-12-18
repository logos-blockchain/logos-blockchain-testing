use std::env;

use testing_framework_env as tf_env;
use tracing::debug;

/// Select the compose image and optional platform, honoring
/// NOMOS_TESTNET_IMAGE.
pub fn resolve_image() -> (String, Option<String>) {
    let image = tf_env::nomos_testnet_image()
        .unwrap_or_else(|| String::from("logos-blockchain-testing:local"));
    let platform = (image == "ghcr.io/logos-co/nomos:testnet").then(|| "linux/amd64".to_owned());
    debug!(image, platform = ?platform, "resolved compose image");
    (image, platform)
}

/// Optional extra hosts entry for host networking.
pub fn host_gateway_entry() -> Option<String> {
    if let Ok(value) = env::var("COMPOSE_RUNNER_HOST_GATEWAY") {
        if value.eq_ignore_ascii_case("disable") || value.is_empty() {
            return None;
        }
        return Some(value);
    }

    if let Ok(gateway) = env::var("DOCKER_HOST_GATEWAY") {
        if !gateway.is_empty() {
            return Some(format!("host.docker.internal:{gateway}"));
        }
    }

    Some("host.docker.internal:host-gateway".into())
}
