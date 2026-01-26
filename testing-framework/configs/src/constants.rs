use std::time::Duration;

use testing_framework_env as tf_env;

/// Default cfgsync port used across runners.
pub const DEFAULT_CFGSYNC_PORT: u16 = 4400;

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
pub const DEFAULT_BLEND_NETWORK_PORT: u16 = 3400; //4401;

/// Resolve cfgsync port from `LOGOS_BLOCKCHAIN_CFGSYNC_PORT`, falling back to
/// the default.
pub fn cfgsync_port() -> u16 {
    tf_env::nomos_cfgsync_port().unwrap_or(DEFAULT_CFGSYNC_PORT)
}

/// Default stack assets directory.
pub const DEFAULT_ASSETS_STACK_DIR: &str = "testing-framework/assets/stack";
