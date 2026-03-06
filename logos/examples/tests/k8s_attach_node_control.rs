use std::time::{Duration, Instant};

use anyhow::{Error, Result, anyhow};
use lb_ext::{CoreBuilderExt as _, LbcExtEnv, LbcK8sDeployer, ScenarioBuilder};
use lb_framework::NodeHttpClient;
use testing_framework_core::scenario::{Deployer as _, Runner};
use testing_framework_runner_k8s::{K8sDeploymentMetadata, K8sRunnerError};
use tokio::time::sleep;

#[tokio::test]
#[ignore = "requires k8s cluster access and mutates k8s runtime state"]
async fn k8s_attach_mode_queries_node_api_opt_in() -> Result<()> {
    let managed = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .with_run_duration(Duration::from_secs(5))
        .build()?;

    let deployer = LbcK8sDeployer::default();
    let (_managed_runner, metadata): (Runner<LbcExtEnv>, K8sDeploymentMetadata) =
        match deployer.deploy_with_metadata(&managed).await {
            Ok(result) => result,
            Err(K8sRunnerError::ClientInit { .. }) => return Ok(()),
            Err(error) => return Err(Error::new(error)),
        };

    let attach_source = metadata.attach_source().map_err(|err| anyhow!("{err}"))?;
    let attached = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .with_run_duration(Duration::from_secs(5))
        .with_attach_source(attach_source)
        .build()?;

    let attached_runner: Runner<LbcExtEnv> = match deployer.deploy(&attached).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { .. }) => return Ok(()),
        Err(error) => return Err(Error::new(error)),
    };

    if attached_runner.context().node_clients().is_empty() {
        return Err(anyhow!("k8s attach resolved no node clients"));
    }

    for node_client in attached_runner.context().node_clients().snapshot() {
        wait_until_node_api_ready(&node_client, Duration::from_secs(20)).await?;
    }

    Ok(())
}

async fn wait_until_node_api_ready(client: &NodeHttpClient, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;

    loop {
        if client.consensus_info().await.is_ok() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for attached node api readiness at {}",
                client.base_url()
            ));
        }

        sleep(Duration::from_millis(500)).await;
    }
}
