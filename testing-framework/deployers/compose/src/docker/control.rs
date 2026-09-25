use std::{path::Path, time::Duration};

use testing_framework_core::{
    adjust_timeout,
    scenario::{
        Application, DynError, ExistingCluster, NodeAccess, NodeControl, NodeControlHandle,
    },
};
use tokio::{process::Command, time::timeout};
use tracing::info;

use crate::{
    docker::{
        attached::{discover_service_container_id, discover_service_node_access},
        commands::{ComposeCommandError, run_docker_command},
    },
    env::discovered_node_access,
    errors::ComposeRunnerError,
    infrastructure::{
        ports::{NodeContainerPorts, NodeHostPorts, compose_runner_host},
        project::ComposeProject,
    },
};

const COMPOSE_RESTART_TIMEOUT: Duration = Duration::from_secs(120);
const COMPOSE_RESTART_DESCRIPTION: &str = "docker compose restart";
const DOCKER_CONTAINER_RESTART_DESCRIPTION: &str = "docker container restart";
const DOCKER_CONTAINER_STOP_DESCRIPTION: &str = "docker container stop";

pub async fn restart_compose_service(
    compose_file: &Path,
    project_name: &str,
    service: &str,
) -> Result<(), ComposeRunnerError> {
    let command = compose_restart_command(compose_file, project_name, service);

    info!(
        service,
        project = project_name,
        compose_file = %compose_file.display(),
        "restarting compose service"
    );

    run_docker_command(
        command,
        adjust_timeout(COMPOSE_RESTART_TIMEOUT),
        COMPOSE_RESTART_DESCRIPTION,
    )
    .await
    .map_err(ComposeRunnerError::Compose)
}

pub async fn restart_attached_compose_service(
    project_name: &str,
    service: &str,
) -> Result<(), DynError> {
    let container_id = discover_service_container_id(project_name, service).await?;
    let command = docker_container_command("restart", &container_id);

    info!(
        service,
        project = project_name,
        container = container_id,
        "restarting attached compose service"
    );

    run_docker_action(
        command,
        DOCKER_CONTAINER_RESTART_DESCRIPTION,
        adjust_timeout(COMPOSE_RESTART_TIMEOUT),
    )
    .await
}

pub async fn stop_attached_compose_service(
    project_name: &str,
    service: &str,
) -> Result<(), DynError> {
    let container_id = discover_service_container_id(project_name, service).await?;
    let command = docker_container_command("stop", &container_id);

    info!(
        service,
        project = project_name,
        container = container_id,
        "stopping attached compose service"
    );

    run_docker_action(
        command,
        DOCKER_CONTAINER_STOP_DESCRIPTION,
        adjust_timeout(COMPOSE_RESTART_TIMEOUT),
    )
    .await
}

fn compose_restart_command(compose_file: &Path, project_name: &str, service: &str) -> Command {
    let mut command = Command::new("docker");
    command
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project_name)
        .arg("restart")
        .arg(service);
    command
}

fn docker_container_command(action: &str, container_id: &str) -> Command {
    let mut command = Command::new("docker");
    command.arg(action).arg(container_id);
    command
}

async fn run_docker_action(
    mut command: Command,
    description: &str,
    timeout_duration: Duration,
) -> Result<(), DynError> {
    match timeout(timeout_duration, command.output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                return Ok(());
            }

            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            Err(format!(
                "{description} failed with status {}: {stderr}",
                output.status
            )
            .into())
        }
        Ok(Err(source)) => Err(format!("{description} failed to spawn: {source}").into()),
        Err(_) => {
            let compose_timeout = ComposeCommandError::Timeout {
                command: description.to_owned(),
                timeout: timeout_duration,
                stdout: String::new(),
                stderr: String::new(),
            };

            Err(compose_timeout.into())
        }
    }
}

/// Compose-specific node control handle for restarting nodes.
pub struct ComposeNodeControl {
    pub(crate) project: ComposeProject,
    pub(crate) node_names: Vec<String>,
    pub(crate) host: String,
    pub(crate) container_ports: Vec<NodeContainerPorts>,
}

#[async_trait::async_trait]
impl NodeControl for ComposeNodeControl {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.project
            .restart_service(name)
            .await
            .map_err(|err| format!("node restart failed: {err}").into())
    }

    fn node_names(&self) -> Vec<String> {
        self.node_names.clone()
    }

    async fn node_access(&self, name: &str) -> Result<NodeAccess, DynError> {
        let index = self
            .node_names
            .iter()
            .position(|node| node == name)
            .ok_or_else(|| format!("unknown compose node '{name}'"))?;
        let ports = self
            .container_ports
            .get(index)
            .ok_or_else(|| format!("no container ports found for compose node '{name}'"))?;
        let host_ports = NodeHostPorts {
            api: self.project.resolve_service_port(name, ports.api).await?,
            testing: self
                .project
                .resolve_service_port(name, ports.testing)
                .await?,
        };
        Ok(discovered_node_access(&self.host, &host_ports))
    }
}

impl<E: Application> NodeControlHandle<E> for ComposeNodeControl {}

/// Node control handle for compose existing-cluster mode.
pub struct ComposeAttachedNodeControl {
    pub(crate) project_name: String,
    pub(crate) node_names: Vec<String>,
    host: String,
}

impl ComposeAttachedNodeControl {
    pub fn try_from_existing_cluster(
        source: &ExistingCluster,
        node_names: Vec<String>,
    ) -> Result<Self, DynError> {
        let Some(project_name) = source
            .compose_project()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Err("attached compose node control requires explicit project name".into());
        };

        Ok(Self {
            project_name: project_name.to_owned(),
            node_names,
            host: compose_runner_host(),
        })
    }
}

#[async_trait::async_trait]
impl NodeControl for ComposeAttachedNodeControl {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        restart_attached_compose_service(&self.project_name, name)
            .await
            .map_err(|source| format!("node restart failed for service '{name}': {source}").into())
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        stop_attached_compose_service(&self.project_name, name)
            .await
            .map_err(|source| format!("node stop failed for service '{name}': {source}").into())
    }

    fn node_names(&self) -> Vec<String> {
        self.node_names.clone()
    }

    async fn node_access(&self, name: &str) -> Result<NodeAccess, DynError> {
        if !self.node_names.iter().any(|node| node == name) {
            return Err(format!("unknown attached compose node '{name}'").into());
        }
        discover_service_node_access(&self.host, &self.project_name, name).await
    }
}

impl<E: Application> NodeControlHandle<E> for ComposeAttachedNodeControl {}

#[cfg(test)]
mod tests {
    use testing_framework_core::scenario::{ExistingCluster, NodeControl, NodeLaunchOptions};

    use super::{ComposeAttachedNodeControl, ComposeNodeControl};
    use crate::infrastructure::{ports::NodeContainerPorts, project::ComposeProject};

    fn managed_control() -> ComposeNodeControl {
        ComposeNodeControl {
            project: ComposeProject::new("compose.yml".into(), "test", ".".into()),
            node_names: vec!["alpha-node-7".into(), "alpha-node-2".into()],
            host: "192.0.2.4".into(),
            container_ports: vec![
                NodeContainerPorts {
                    index: 0,
                    api: 19090,
                    testing: 19091,
                },
                NodeContainerPorts {
                    index: 1,
                    api: 19092,
                    testing: 19093,
                },
            ],
        }
    }

    #[tokio::test]
    async fn unknown_nodes_fail_before_invoking_docker() {
        let managed = managed_control();
        let source =
            ExistingCluster::for_compose_services("test".into(), vec!["alpha-node-7".into()]);
        let attached = ComposeAttachedNodeControl::try_from_existing_cluster(
            &source,
            vec!["alpha-node-7".into()],
        )
        .unwrap();

        for control in [&managed as &dyn NodeControl, &attached as &dyn NodeControl] {
            let error = control.node_access("other-node").await.unwrap_err();
            assert!(error.to_string().contains("unknown"));
            assert!(error.to_string().contains("other-node"));
        }
    }

    #[tokio::test]
    async fn common_control_does_not_add_unsupported_compose_operations() {
        let managed = managed_control();
        let source =
            ExistingCluster::for_compose_services("test".into(), vec!["alpha-node-7".into()]);
        let attached = ComposeAttachedNodeControl::try_from_existing_cluster(
            &source,
            vec!["alpha-node-7".into()],
        )
        .unwrap();

        for control in [&managed as &dyn NodeControl, &attached as &dyn NodeControl] {
            assert!(
                control
                    .start_node("alpha-node-7")
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("not supported")
            );
            assert!(
                control
                    .start_node_with("alpha-node-7", NodeLaunchOptions::default())
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("not supported")
            );
            assert!(
                control
                    .restart_node_with("alpha-node-7", NodeLaunchOptions::default())
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("not supported")
            );
            assert!(
                control
                    .wait_node_ready("alpha-node-7")
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("not supported")
            );
        }
        assert!(
            managed
                .stop_node("alpha-node-7")
                .await
                .unwrap_err()
                .to_string()
                .contains("not supported")
        );
    }
}
