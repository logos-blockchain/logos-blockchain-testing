use queue_runtime_workloads::{
    QueueDslExt as _, QueueRunExt as _, QueueScenario, RestartChaosBuilderExt as _,
};
use testing_framework_core::scenario::DynError;

#[tokio::test]
async fn dsl_restart_scenario_converges() -> Result<(), DynError> {
    QueueScenario::nodes(3)
        .produce(100)
        .rate_per_sec(50)
        .done()
        .restart_nodes_randomly()
        .every_secs(4, 8)
        .cooldown_secs(10)
        .excluding_nodes(["node-0"])
        .done()
        .expect_converged(100)
        .within_secs(30)
        .run_secs(30)
        .await
}
