use std::env;

/// Default cfgsync port used across runners.
pub const DEFAULT_CFGSYNC_PORT: u16 = 4400;

/// Default container path for KZG parameters (compose/k8s mount point).
pub const DEFAULT_KZG_CONTAINER_PATH: &str = "/kzgrs_test_params/kzgrs_test_params";

/// Default host-relative directory for KZG assets.
pub const DEFAULT_KZG_HOST_DIR: &str = "testing-framework/assets/stack/kzgrs_test_params";

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
