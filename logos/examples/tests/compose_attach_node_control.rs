use std::time::Duration;

use anyhow::{Result, anyhow};
use lb_ext::{CoreBuilderExt as _, LbcComposeDeployer, LbcExtEnv, ScenarioBuilder};
use testing_framework_core::scenario::{AttachSource, Deployer as _, Runner};
use testing_framework_runner_compose::{ComposeDeploymentMetadata, ComposeRunnerError};
use tokio::process::Command;

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

    let project_name = metadata
        .project_name()
        .ok_or_else(|| anyhow!("compose deployment metadata has no project name"))?
        .to_owned();
    let attach_source = AttachSource::compose(vec![]).with_project(project_name.clone());

    let attached = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(5))
        .with_attach_source(attach_source)
        .build()?;

    let attached_runner: Runner<LbcExtEnv> = match deployer.deploy(&attached).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
        Err(error) => return Err(anyhow::Error::new(error)),
    };

    let control = attached_runner
        .context()
        .node_control()
        .ok_or_else(|| anyhow!("attached compose node control is unavailable"))?;

    let services: Vec<String> = attached_runner
        .context()
        .borrowed_nodes()
        .into_iter()
        .map(|node| node.identity)
        .collect();

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

    let container_id = container_id
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("no running container found for service '{service}'"))?;

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

async fn wait_until_service_restarted(
    project: &str,
    service: &str,
    previous_started_at: &str,
    timeout: Duration,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;

    loop {
        let started_at = service_started_at(project, service).await?;

        if started_at != previous_started_at {
            return Ok(());
        }

        if std::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for restarted compose service '{service}'"
            ));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
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
