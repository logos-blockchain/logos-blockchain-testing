use testing_framework_env as tf_env;

/// Default cfgsync port used across extension runners.
pub const DEFAULT_CFGSYNC_PORT: u16 = 4400;

/// Default stack assets directory used by k8s compose assets discovery.
pub const DEFAULT_ASSETS_STACK_DIR: &str = "logos/infra/assets/stack";

/// Resolve cfgsync port from `LOGOS_BLOCKCHAIN_CFGSYNC_PORT`, falling back to
/// the default.
pub fn cfgsync_port() -> u16 {
    tf_env::nomos_cfgsync_port().unwrap_or(DEFAULT_CFGSYNC_PORT)
}
