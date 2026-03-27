use std::{
    path::{Path, PathBuf},
    process::Command as StdCommand,
    time::Duration,
};

use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::{docker::commands::run_docker_command, env::ConfigServerHandle};

#[derive(Clone, Debug)]
pub struct DockerPortBinding {
    pub host_port: u16,
    pub container_port: u16,
}

impl DockerPortBinding {
    #[must_use]
    pub fn tcp(host_port: u16, container_port: u16) -> Self {
        Self {
            host_port,
            container_port,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DockerVolumeMount {
    pub host_path: PathBuf,
    pub container_path: String,
    pub read_only: bool,
}

impl DockerVolumeMount {
    #[must_use]
    pub fn read_only(host_path: PathBuf, container_path: String) -> Self {
        Self {
            host_path,
            container_path,
            read_only: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DockerConfigServerSpec {
    pub container_name: String,
    pub network: String,
    pub network_alias: Option<String>,
    pub platform: Option<String>,
    pub workdir: Option<String>,
    pub entrypoint: String,
    pub image: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub ports: Vec<DockerPortBinding>,
    pub mounts: Vec<DockerVolumeMount>,
}

impl DockerConfigServerSpec {
    #[must_use]
    pub fn new(container_name: String, network: String, entrypoint: String, image: String) -> Self {
        Self {
            container_name,
            network,
            network_alias: None,
            platform: None,
            workdir: None,
            entrypoint,
            image,
            args: Vec::new(),
            env: Vec::new(),
            ports: Vec::new(),
            mounts: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_network_alias(mut self, alias: String) -> Self {
        self.network_alias = Some(alias);
        self
    }

    #[must_use]
    pub fn with_platform(mut self, platform: Option<String>) -> Self {
        self.platform = platform;
        self
    }

    #[must_use]
    pub fn with_workdir(mut self, workdir: String) -> Self {
        self.workdir = Some(workdir);
        self
    }

    #[must_use]
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    #[must_use]
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.env = env;
        self
    }

    #[must_use]
    pub fn with_ports(mut self, ports: Vec<DockerPortBinding>) -> Self {
        self.ports = ports;
        self
    }

    #[must_use]
    pub fn with_mounts(mut self, mounts: Vec<DockerVolumeMount>) -> Self {
        self.mounts = mounts;
        self
    }
}

#[derive(Debug)]
pub struct DockerConfigServerHandle {
    name: String,
    stopped: bool,
}

impl DockerConfigServerHandle {
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            name,
            stopped: false,
        }
    }
}

impl ConfigServerHandle for DockerConfigServerHandle {
    fn shutdown(&mut self) {
        if self.stopped {
            return;
        }

        let name = self.name.clone();
        let status = StdCommand::new("docker")
            .arg("rm")
            .arg("-f")
            .arg(&name)
            .status();
        match status {
            Ok(status) if status.success() => {
                debug!(container = name, "removed config server container");
            }
            Ok(status) => {
                warn!(container = name, status = ?status, "failed to remove config server container");
            }
            Err(err) => {
                warn!(container = name, error = ?err, "failed to spawn docker rm for config server container");
            }
        }

        self.stopped = true;
    }

    fn mark_preserved(&mut self) {
        self.stopped = true;
    }

    fn container_name(&self) -> Option<&str> {
        Some(self.name.as_str())
    }
}

pub async fn start_docker_config_server(
    spec: &DockerConfigServerSpec,
    timeout: Duration,
    description: &str,
) -> Result<DockerConfigServerHandle, crate::docker::commands::ComposeCommandError> {
    run_docker_command(build_docker_run_command(spec), timeout, description).await?;
    info!(container = %spec.container_name, image = %spec.image, "docker config server started");
    Ok(DockerConfigServerHandle::new(spec.container_name.clone()))
}

fn build_docker_run_command(spec: &DockerConfigServerSpec) -> Command {
    let mut command = Command::new("docker");
    command
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(&spec.container_name)
        .arg("--network")
        .arg(&spec.network);

    if let Some(platform) = &spec.platform {
        command.arg("--platform").arg(platform);
    }

    command.arg("--entrypoint").arg(&spec.entrypoint);

    if let Some(alias) = &spec.network_alias {
        command.arg("--network-alias").arg(alias);
    }

    if let Some(workdir) = &spec.workdir {
        command.arg("--workdir").arg(workdir);
    }

    for (key, value) in &spec.env {
        command.arg("-e").arg(format!("{key}={value}"));
    }

    for port in &spec.ports {
        command
            .arg("-p")
            .arg(format!("{}:{}", port.host_port, port.container_port));
    }

    for mount in &spec.mounts {
        command
            .arg("-v")
            .arg(volume_mount_arg(mount, mount.host_path.as_path()));
    }

    command.arg(&spec.image).args(&spec.args);
    command
}

fn volume_mount_arg(mount: &DockerVolumeMount, host_path: &Path) -> String {
    let resolved_host_path = host_path
        .canonicalize()
        .unwrap_or_else(|_| host_path.to_path_buf());
    let mode = if mount.read_only { ":ro" } else { "" };
    format!(
        "{}:{}{}",
        resolved_host_path.display(),
        mount.container_path,
        mode
    )
}
