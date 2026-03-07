use std::time::{Duration, Instant};

use anyhow::{Error, Result, anyhow};
use lb_ext::{CoreBuilderExt as _, LbcComposeDeployer, LbcExtEnv, ScenarioBuilder};
use testing_framework_core::scenario::{Deployer as _, Runner};
use testing_framework_runner_compose::{ComposeDeploymentMetadata, ComposeRunnerError};
use tokio::{process::Command, time::sleep};

#[tokio::test]
#[ignore = "requires Docker and mutates compose runtime state"]
async fn compose_attach_mode_queries_node_api_opt_in() -> Result<()> {
    let managed = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .with_run_duration(Duration::from_secs(5))
        .build()?;

    let deployer = LbcComposeDeployer::default();
    let (_managed_runner, metadata): (Runner<LbcExtEnv>, ComposeDeploymentMetadata) =
        match deployer.deploy_with_metadata(&managed).await {
            Ok(result) => result,
            Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
            Err(error) => return Err(Error::new(error)),
        };

    let attach_source = metadata.attach_source().map_err(|err| anyhow!("{err}"))?;
    let attached = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .with_run_duration(Duration::from_secs(5))
        .with_attach_source(attach_source)
        .build()?;

    let attached_runner: Runner<LbcExtEnv> = match deployer.deploy(&attached).await {
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
            Err(error) => return Err(Error::new(error)),
        };

    let project_name = metadata
        .project_name()
        .ok_or_else(|| anyhow!("compose deployment metadata has no project name"))?
        .to_owned();

    let attach_source = metadata.attach_source().map_err(|err| anyhow!("{err}"))?;

    let attached = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(5))
        .with_attach_source(attach_source)
        .build()?;

    let attached_runner: Runner<LbcExtEnv> = match deployer.deploy(&attached).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
        Err(error) => return Err(Error::new(error)),
    };

    let control = attached_runner
        .context()
        .node_control()
        .ok_or_else(|| anyhow!("attached compose node control is unavailable"))?;

    let services = discover_attached_services(&project_name).await?;

    if services.is_empty() {
        return Err(anyhow!("attached compose runner discovered no services"));
    }

    for service in services {
        let pre_restart_started_at = service_started_at(&project_name, &service).await?;

        control
            .restart_node(&service)
            .await
            .map_err(|err| anyhow!("attached restart failed for {service}: {err}"))?;

        wait_until_service_restarted(
            &project_name,
            &service,
            &pre_restart_started_at,
            Duration::from_secs(30),
        )
        .await?;
    }

    Ok(())
}

async fn service_started_at(project: &str, service: &str) -> Result<String> {
    let container_id = run_docker(&[
        "ps",
        "--filter",
        &format!("label=com.docker.compose.project={project}"),
        "--filter",
        &format!("label=com.docker.compose.service={service}"),
        "--format",
        "{{.ID}}",
    ])
    .await?;

    let container_ids: Vec<&str> = container_id
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();

    let container_id = match container_ids.as_slice() {
        [] => {
            return Err(anyhow!(
                "no running container found for service '{service}'"
            ));
        }
        [id] => *id,
        _ => {
            return Err(anyhow!(
                "multiple running containers found for service '{service}'"
            ));
        }
    };

    let started_at =
        run_docker(&["inspect", "--format", "{{.State.StartedAt}}", container_id]).await?;

    let started_at = started_at.trim().to_owned();

    if started_at.is_empty() {
        return Err(anyhow!(
            "docker inspect returned empty StartedAt for service '{service}'"
        ));
    }

    Ok(started_at)
}

async fn discover_attached_services(project: &str) -> Result<Vec<String>> {
    let output = run_docker(&[
        "ps",
        "--filter",
        &format!("label=com.docker.compose.project={project}"),
        "--filter",
        "label=testing-framework.node=true",
        "--format",
        "{{.Label \"com.docker.compose.service\"}}",
    ])
    .await?;

    let mut services: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    services.sort();
    services.dedup();

    if services.is_empty() {
        return Err(anyhow!(
            "attached compose runner discovered no labeled services"
        ));
    }

    Ok(services)
}

async fn wait_until_service_restarted(
    project: &str,
    service: &str,
    previous_started_at: &str,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;

    loop {
        let started_at = service_started_at(project, service).await?;

        if started_at != previous_started_at {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for restarted compose service '{service}'"
            ));
        }

        sleep(Duration::from_millis(500)).await;
    }
}

async fn run_docker(args: &[&str]) -> Result<String> {
    let output = Command::new("docker").args(args).output().await?;

    if !output.status.success() {
        return Err(anyhow!(
            "docker {} failed with status {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
