use std::env;

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
