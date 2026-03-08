use std::time::Duration;

use anyhow::{Error, Result, anyhow};
use lb_ext::{CoreBuilderExt as _, LbcComposeDeployer, LbcExtEnv, ScenarioBuilder};
use testing_framework_core::scenario::{Deployer as _, Runner};
use testing_framework_runner_compose::{ComposeDeploymentMetadata, ComposeRunnerError};

#[tokio::test]
#[ignore = "requires Docker and mutates compose runtime state"]
async fn compose_attach_mode_queries_node_api_opt_in() -> Result<()> {
    let managed = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .with_run_duration(Duration::from_secs(5))
        .build()?;

    let managed_deployer = LbcComposeDeployer::default();
    let (_managed_runner, metadata): (Runner<LbcExtEnv>, ComposeDeploymentMetadata) =
        match managed_deployer.deploy_with_metadata(&managed).await {
            Ok(result) => result,
            Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
            Err(error) => return Err(Error::new(error)),
        };

    let attach_source = metadata.attach_source().map_err(|err| anyhow!("{err}"))?;
    let attached = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .with_run_duration(Duration::from_secs(5))
        .with_attach_source(attach_source)
        .build()?;

    let attached_deployer = LbcComposeDeployer::default();
    let attached_runner: Runner<LbcExtEnv> = match attached_deployer.deploy(&attached).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
        Err(error) => return Err(Error::new(error)),
    };

    attached_runner
        .wait_network_ready()
        .await
        .map_err(|err| anyhow!("compose attached runner readiness failed: {err}"))?;

    if attached_runner.context().node_clients().is_empty() {
        return Err(anyhow!("compose attach resolved no node clients"));
    }

    for node_client in attached_runner.context().node_clients().snapshot() {
        node_client.consensus_info().await.map_err(|err| {
            anyhow!(
                "attached node api query failed at {}: {err}",
                node_client.base_url()
            )
        })?;
    }

    Ok(())
}
