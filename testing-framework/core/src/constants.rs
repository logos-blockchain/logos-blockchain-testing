use std::{env, time::Duration};

/// Default cfgsync port used across runners.
pub const DEFAULT_CFGSYNC_PORT: u16 = 4400;

/// Default container path for KZG parameters (compose/k8s mount point).
pub const DEFAULT_KZG_CONTAINER_PATH: &str = "/kzgrs_test_params/kzgrs_test_params";

/// Default host-relative directory for KZG assets.
pub const DEFAULT_KZG_HOST_DIR: &str = "testing-framework/assets/stack/kzgrs_test_params";

/// Default HTTP probe interval used across readiness checks.
pub const DEFAULT_HTTP_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Default node HTTP timeout when probing endpoints.
pub const DEFAULT_NODE_HTTP_TIMEOUT: Duration = Duration::from_secs(240);

/// Default node HTTP timeout when probing NodePort endpoints.
pub const DEFAULT_NODE_HTTP_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Default Kubernetes deployment readiness timeout.
pub const DEFAULT_K8S_DEPLOYMENT_TIMEOUT: Duration = Duration::from_secs(180);

/// Default API port used by nodes.
pub const DEFAULT_API_PORT: u16 = 18080;

/// Default testing HTTP port used by nodes.
pub const DEFAULT_TESTING_HTTP_PORT: u16 = 18081;

/// Default libp2p network port.
pub const DEFAULT_LIBP2P_NETWORK_PORT: u16 = 3000;

/// Default DA network port.
pub const DEFAULT_DA_NETWORK_PORT: u16 = 3300;

/// Default blend network port.
pub const DEFAULT_BLEND_NETWORK_PORT: u16 = 4401;

/// Resolve cfgsync port from `NOMOS_CFGSYNC_PORT`, falling back to the default.
pub fn cfgsync_port() -> u16 {
    env::var("NOMOS_CFGSYNC_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_CFGSYNC_PORT)
}

/// Resolve container KZG path from `NOMOS_KZG_CONTAINER_PATH`, falling back to
/// the default.
pub fn kzg_container_path() -> String {
    env::var("NOMOS_KZG_CONTAINER_PATH").unwrap_or_else(|_| DEFAULT_KZG_CONTAINER_PATH.to_string())
}
