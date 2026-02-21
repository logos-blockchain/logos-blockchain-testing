use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use lb_framework::{CoreBuilderExt as _, LbcEnv, LbcLocalDeployer, ScenarioBuilder};
use testing_framework_core::scenario::{
    Deployer, DynError, Expectation, RunContext, ScenarioError,
};
use tracing_subscriber::fmt::try_init;

const FAIL_AFTER_CHECKS: usize = 2;

struct FailFastDuringCaptureExpectation {
    checks: usize,
}

impl Default for FailFastDuringCaptureExpectation {
    fn default() -> Self {
        Self { checks: 0 }
    }
}

#[async_trait]
impl Expectation<LbcEnv> for FailFastDuringCaptureExpectation {
    fn name(&self) -> &str {
        "fail_fast_during_capture"
    }

    async fn check_during_capture(&mut self, _ctx: &RunContext<LbcEnv>) -> Result<(), DynError> {
        self.checks += 1;
        if self.checks >= FAIL_AFTER_CHECKS {
            return Err(format!(
                "intentional fail-fast trigger after {} capture checks",
                self.checks
            )
            .into());
        }

        Ok(())
    }

    async fn evaluate(&mut self, _ctx: &RunContext<LbcEnv>) -> Result<(), DynError> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires local node binary and open ports"]
async fn expectation_can_fail_fast_during_capture() -> Result<()> {
    let _ = try_init();

    let mut scenario = ScenarioBuilder::deployment_with(|topology| topology.with_node_count(1))
        .with_run_duration(Duration::from_secs(30))
        .with_expectation(FailFastDuringCaptureExpectation::default())
        .build()?;

    let deployer = LbcLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;

    match runner.run(&mut scenario).await {
        Err(ScenarioError::ExpectationFailedDuringCapture(_)) => Ok(()),
        Err(other) => Err(anyhow!("unexpected scenario error: {other}")),
        Ok(_) => Err(anyhow!("expected fail-fast capture error, run succeeded")),
    }
}
