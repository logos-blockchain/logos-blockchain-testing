use std::{
    env, fs,
    path::{Path, PathBuf},
};

use cucumber::World;
use cucumber_ext::TestingFrameworkWorld;
use tracing_subscriber::{EnvFilter, fmt};

const FEATURES_PATH: &str = "examples/cucumber/features";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Host,
    Compose,
}

fn set_default_env(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        // SAFETY: Used as an early-run default. Prefer setting env vars in the
        // shell for multi-threaded runs.
        unsafe {
            std::env::set_var(key, value);
        }
    }
}

fn is_compose(
    feature: &cucumber::gherkin::Feature,
    scenario: &cucumber::gherkin::Scenario,
) -> bool {
    scenario.tags.iter().any(|tag| tag == "compose")
        || feature.tags.iter().any(|tag| tag == "compose")
}

pub fn init_logging_defaults() {
    set_default_env("POL_PROOF_DEV_MODE", "true");
    set_default_env("NOMOS_TESTS_KEEP_LOGS", "1");
    set_default_env("NOMOS_LOG_LEVEL", "info");
    set_default_env("RUST_LOG", "info");
}

pub fn init_node_log_dir_defaults(mode: Mode) {
    if env::var_os("NOMOS_LOG_DIR").is_some() {
        return;
    }

    let host_dir = repo_root().join("tmp").join("node-logs");
    let _ = fs::create_dir_all(&host_dir);

    match mode {
        Mode::Host => set_default_env("NOMOS_LOG_DIR", &host_dir.display().to_string()),
        Mode::Compose => set_default_env("NOMOS_LOG_DIR", "/tmp/node-logs"),
    }
}

fn repo_root() -> PathBuf {
    env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(Path::to_path_buf)
        })
        .expect("repo root must be discoverable from CARGO_WORKSPACE_DIR or CARGO_MANIFEST_DIR")
}

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).with_target(true).try_init();
}

pub async fn run(mode: Mode) {
    TestingFrameworkWorld::cucumber()
        .with_default_cli()
        .max_concurrent_scenarios(Some(1))
        .filter_run(FEATURES_PATH, move |feature, _, scenario| match mode {
            Mode::Host => !is_compose(feature, scenario),
            Mode::Compose => is_compose(feature, scenario),
        })
        .await;
}
