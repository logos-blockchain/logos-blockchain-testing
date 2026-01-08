use cucumber::World;
use cucumber_ext::{DeployerKind, TestingFrameworkWorld};
use runner_examples::defaults::{init_logging_defaults, init_node_log_dir_defaults, init_tracing};

#[tokio::main]
async fn main() {
    init_logging_defaults();
    init_node_log_dir_defaults(DeployerKind::Compose);
    init_tracing();

    TestingFrameworkWorld::run("examples/cucumber/features/compose_smoke.feature").await;
}
