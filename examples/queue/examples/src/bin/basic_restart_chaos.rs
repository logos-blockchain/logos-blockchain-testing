use std::time::Duration;

use async_trait::async_trait;
use queue_runtime_ext::QueueLocalDeployer;
use queue_runtime_workloads::{
    QueueBuilderExt, QueueConverges, QueueProduceWorkload, QueueScenarioBuilder, QueueTopology,
};
use testing_framework_core::{
    scenario::{Deployer, DynError, RunContext, Workload},
    topology::DeploymentDescriptor,
};
use tracing::info;

#[derive(Clone)]
struct FixedRestartChaosWorkload {
    restarts: usize,
    delay: Duration,
}

impl FixedRestartChaosWorkload {
    const fn new(restarts: usize, delay: Duration) -> Self {
        Self { restarts, delay }
    }
}

#[async_trait]
impl Workload<queue_runtime_workloads::QueueEnv> for FixedRestartChaosWorkload {
    fn name(&self) -> &str {
        "fixed_restart_chaos"
    }

    async fn start(
        &self,
        ctx: &RunContext<queue_runtime_workloads::QueueEnv>,
    ) -> Result<(), DynError> {
        let Some(control) = ctx.node_control() else {
            return Err("fixed restart chaos requires node control".into());
        };

        let node_count = ctx.descriptors().node_count();
        if node_count == 0 {
            return Err("fixed restart chaos requires at least one node".into());
        }

        for step in 0..self.restarts {
            tokio::time::sleep(self.delay).await;
            let target_index = if node_count > 1 {
                (step % (node_count - 1)) + 1
            } else {
                0
            };
            let target = format!("node-{target_index}");
            info!(step, %target, "triggering controlled chaos restart");
            control.restart_node(&target).await?;
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = QueueScenarioBuilder::deployment_with(|_| QueueTopology::new(3))
        .enable_node_control()
        .with_workload(FixedRestartChaosWorkload::new(3, Duration::from_secs(8)))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            QueueProduceWorkload::new()
                .operations(400)
                .rate_per_sec(40)
                .payload_prefix("queue-chaos"),
        )
        .with_expectation(QueueConverges::new(200).timeout(Duration::from_secs(30)))
        .build()?;

    let deployer = QueueLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}
