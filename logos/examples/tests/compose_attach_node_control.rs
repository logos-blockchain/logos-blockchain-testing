use std::{
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Result, anyhow};
use lb_ext::{CoreBuilderExt as _, LbcComposeDeployer, LbcExtEnv, ScenarioBuilder};
use testing_framework_core::scenario::{AttachSource, Deployer as _, Runner};
use testing_framework_runner_compose::ComposeRunnerError;

#[tokio::test]
#[ignore = "requires Docker and mutates compose runtime state"]
async fn compose_attach_mode_restart_node_opt_in() -> Result<()> {
    let managed = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(5))
        .build()?;

    let deployer = LbcComposeDeployer::default();
    let (managed_runner, metadata): (Runner<LbcExtEnv>, _) =
        match deployer.deploy_with_metadata(&managed).await {
            Ok(result) => result,
            Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
            Err(error) => return Err(anyhow::Error::new(error)),
        };
    let managed_client = managed_runner
        .context()
        .node_clients()
        .snapshot()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("managed compose runner returned no node clients"))?;
    managed_client
        .base_url()
        .port()
        .ok_or_else(|| anyhow!("managed node base url has no port"))?;

    let project_name = metadata
        .project_name
        .ok_or_else(|| anyhow!("compose metadata did not include project name"))?;

    let attached = ScenarioBuilder::deployment_with(|d| d.with_node_count(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(5))
        .with_attach_source(
            AttachSource::compose(vec!["node-0".to_owned()]).with_project(project_name.clone()),
        )
        .build()?;

    let attached_runner: Runner<LbcExtEnv> = match deployer.deploy(&attached).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => return Ok(()),
        Err(error) => return Err(anyhow::Error::new(error)),
    };
    let pre_restart_container = discover_compose_service_container(&project_name, "node-0")?;
    let pre_restart_started_at = inspect_container_started_at(&pre_restart_container)?;
    let control = attached_runner
        .context()
        .node_control()
        .ok_or_else(|| anyhow!("attached compose node control is unavailable"))?;
    control
        .restart_node("node-0")
        .await
        .map_err(|err| anyhow!("attached restart failed: {err}"))?;

    wait_until_container_restarted(
        &project_name,
        "node-0",
        &pre_restart_started_at,
        Duration::from_secs(30),
    )?;

    Ok(())
}
fn discover_compose_service_container(project: &str, service: &str) -> Result<String> {
    let container = run_docker_capture([
        "ps",
        "--filter",
        &format!("label=com.docker.compose.project={project}"),
        "--filter",
        &format!("label=com.docker.compose.service={service}"),
        "--format",
        "{{.ID}}",
    ])?;
    let mut lines = container
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(container_id) = lines.next() else {
        return Err(anyhow!(
            "no running container found for compose project '{project}' service '{service}'"
        ));
    };
    if lines.next().is_some() {
        return Err(anyhow!(
            "multiple running containers found for compose project '{project}' service '{service}'"
        ));
    }

    Ok(container_id.to_owned())
}

fn inspect_container_started_at(container_id: &str) -> Result<String> {
    let started_at =
        run_docker_capture(["inspect", "--format", "{{.State.StartedAt}}", container_id])?;
    let started_at = started_at.trim();
    if started_at.is_empty() {
        return Err(anyhow!(
            "docker inspect returned empty StartedAt for container {container_id}"
        ));
    }

    Ok(started_at.to_owned())
}

fn wait_until_container_restarted(
    project: &str,
    service: &str,
    previous_started_at: &str,
    timeout: Duration,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let container_id = discover_compose_service_container(project, service)?;
        let started_at = inspect_container_started_at(&container_id)?;
        if started_at != previous_started_at {
            return Ok(());
        }

        if std::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for restarted container timestamp change: {project}/{service}"
            ));
        }

        thread::sleep(Duration::from_millis(500));
    }
}

fn run_docker_capture<const N: usize>(args: [&str; N]) -> Result<String> {
    let output = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "docker {} failed: status={} stderr={}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
