use std::env;

const NODE_HOST_ENV: &str = "K8S_RUNNER_NODE_HOST";
const KUBE_SERVICE_HOST_ENV: &str = "KUBERNETES_SERVICE_HOST";
use tracing::debug;

/// Returns the hostname or IP used to reach `NodePorts` exposed by the cluster.
/// Prefers `K8S_RUNNER_NODE_HOST`, then the standard `KUBERNETES_SERVICE_HOST`
/// (e.g. `kubernetes.docker.internal` on Docker Desktop), and finally falls
/// back to `127.0.0.1`.
pub fn node_host() -> String {
    if let Ok(host) = env::var(NODE_HOST_ENV) {
        debug!(host, env = NODE_HOST_ENV, "using node host override");
        return host;
    }
    if let Ok(host) = env::var(KUBE_SERVICE_HOST_ENV)
        && !host.is_empty()
    {
        debug!(
            host,
            env = KUBE_SERVICE_HOST_ENV,
            "using kubernetes service host"
        );
        return host;
    }
    debug!("falling back to 127.0.0.1 for node host");
    "127.0.0.1".to_owned()
}
