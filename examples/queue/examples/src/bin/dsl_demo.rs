use queue_runtime_workloads::{
    QueueDslExt as _, QueueRunExt as _, QueueScenario, RestartChaosBuilderExt as _,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    QueueScenario::nodes(5)
        .produce(400)
        .rate_per_sec(40)
        .done()
        .restart_nodes_randomly()
        .every_secs(5, 15)
        .excluding_nodes(["node-0"])
        .done()
        .expect_converged(400)
        .within_secs(60)
        .run_secs(120)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

    Ok(())
}
