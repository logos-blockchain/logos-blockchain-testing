use std::time::Duration;

use anyhow::{Result, anyhow};
use lb_ext::{CoreBuilderExt as _, LbcComposeDeployer, LbcExtEnv, ScenarioBuilder};
use testing_framework_core::scenario::{Deployer as _, Runner};
use testing_framework_runner_compose::{ComposeDeploymentMetadata, ComposeRunnerError};

#[tokio::test]
#[ignore = "requires Docker and mutates compose runtime state"]
async fn compose_attach_mode_restart_node_opt_in() -> Result<()> {
    let managed = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(5))
        .build()?;

    let deployer = LbcComposeDeployer::default();
    let (_managed_runner, metadata): (Runner<LbcExtEnv>, ComposeDeploymentMetadata) =
        match deployer.deploy_with_metadata(&managed).await {
            Ok(result) => result,
            Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
            Err(error) => return Err(anyhow::Error::new(error)),
        };

    let attached = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(5))
        .with_attach_source(
            metadata
                .attach_source_for_services(vec!["node-0".to_owned()])
                .map_err(|err| anyhow!("{err}"))?,
        )
        .build()?;

    let attached_runner: Runner<LbcExtEnv> = match deployer.deploy(&attached).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
        Err(error) => return Err(anyhow::Error::new(error)),
    };

    let pre_restart_started_at = metadata
        .service_started_at("node-0")
        .await
        .map_err(|err| anyhow!("{err}"))?;

    let control = attached_runner
        .context()
        .node_control()
        .ok_or_else(|| anyhow!("attached compose node control is unavailable"))?;

    control
        .restart_node("node-0")
        .await
        .map_err(|err| anyhow!("attached restart failed: {err}"))?;

    metadata
        .wait_until_service_restarted("node-0", &pre_restart_started_at, Duration::from_secs(30))
        .await
        .map_err(|err| anyhow!("{err}"))?;

    Ok(())
}
