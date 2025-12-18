use std::{env, path::PathBuf};

#[must_use]
pub fn slow_test_env() -> bool {
    env::var("SLOW_TEST_ENV").is_ok_and(|s| s == "true")
}

#[must_use]
pub fn debug_tracing() -> bool {
    env::var("NOMOS_TESTS_TRACING").is_ok_and(|val| val.eq_ignore_ascii_case("true"))
}

#[must_use]
pub fn nomos_log_dir() -> Option<PathBuf> {
    env::var("NOMOS_LOG_DIR").ok().map(PathBuf::from)
}

#[must_use]
pub fn nomos_log_level() -> Option<String> {
    env::var("NOMOS_LOG_LEVEL").ok()
}

#[must_use]
pub fn nomos_log_filter() -> Option<String> {
    env::var("NOMOS_LOG_FILTER").ok()
}

#[must_use]
pub fn nomos_use_autonat() -> bool {
    env::var("NOMOS_USE_AUTONAT").is_ok()
}

#[must_use]
pub fn nomos_cfgsync_port() -> Option<u16> {
    env::var("NOMOS_CFGSYNC_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
}

#[must_use]
pub fn nomos_kzg_container_path() -> Option<String> {
    env::var("NOMOS_KZG_CONTAINER_PATH").ok()
}

#[must_use]
pub fn nomos_tests_keep_logs() -> bool {
    env::var("NOMOS_TESTS_KEEP_LOGS").is_ok()
}

#[must_use]
pub fn nomos_testnet_image() -> Option<String> {
    env::var("NOMOS_TESTNET_IMAGE").ok()
}

#[must_use]
pub fn nomos_testnet_image_pull_policy() -> Option<String> {
    env::var("NOMOS_TESTNET_IMAGE_PULL_POLICY").ok()
}

#[must_use]
pub fn nomos_kzg_mode() -> Option<String> {
    env::var("NOMOS_KZG_MODE").ok()
}

#[must_use]
pub fn nomos_kzg_dir_rel() -> Option<String> {
    env::var("NOMOS_KZG_DIR_REL").ok()
}

#[must_use]
pub fn nomos_kzg_file() -> Option<String> {
    env::var("NOMOS_KZG_FILE").ok()
}

#[must_use]
pub fn pol_proof_dev_mode() -> Option<String> {
    env::var("POL_PROOF_DEV_MODE").ok()
}

#[must_use]
pub fn rust_log() -> Option<String> {
    env::var("RUST_LOG").ok()
}

#[must_use]
pub fn nomos_time_backend() -> Option<String> {
    env::var("NOMOS_TIME_BACKEND").ok()
}

#[must_use]
pub fn nomos_kzgrs_params_path() -> Option<String> {
    env::var("NOMOS_KZGRS_PARAMS_PATH").ok()
}

#[must_use]
pub fn nomos_otlp_endpoint() -> Option<String> {
    env::var("NOMOS_OTLP_ENDPOINT").ok()
}

#[must_use]
pub fn nomos_otlp_metrics_endpoint() -> Option<String> {
    env::var("NOMOS_OTLP_METRICS_ENDPOINT").ok()
}
