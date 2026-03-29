use std::{
    io,
    path::PathBuf,
    process::{Output, Stdio},
};

use thiserror::Error;
use tokio::process::Command;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct HelmValueSetting {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct HelmFileSetting {
    pub key: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct HelmInstallSpec {
    pub release: String,
    pub chart_path: PathBuf,
    pub namespace: String,
    pub values_files: Vec<PathBuf>,
    pub set_values: Vec<HelmValueSetting>,
    pub set_files: Vec<HelmFileSetting>,
    pub current_dir: Option<PathBuf>,
    pub wait_timeout: String,
}

impl HelmInstallSpec {
    #[must_use]
    pub fn new(release: String, chart_path: PathBuf, namespace: String) -> Self {
        Self {
            release,
            chart_path,
            namespace,
            values_files: Vec::new(),
            set_values: Vec::new(),
            set_files: Vec::new(),
            current_dir: None,
            wait_timeout: "5m".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HelmReleaseBundle {
    pub chart_path: PathBuf,
    pub values_files: Vec<PathBuf>,
    pub set_values: Vec<HelmValueSetting>,
    pub set_files: Vec<HelmFileSetting>,
    pub current_dir: Option<PathBuf>,
    pub wait_timeout: String,
}

impl HelmReleaseBundle {
    #[must_use]
    pub fn new(chart_path: PathBuf) -> Self {
        Self {
            chart_path,
            values_files: Vec::new(),
            set_values: Vec::new(),
            set_files: Vec::new(),
            current_dir: None,
            wait_timeout: "5m".to_string(),
        }
    }

    #[must_use]
    pub fn install_spec(&self, release: String, namespace: String) -> HelmInstallSpec {
        HelmInstallSpec {
            release,
            chart_path: self.chart_path.clone(),
            namespace,
            values_files: self.values_files.clone(),
            set_values: self.set_values.clone(),
            set_files: self.set_files.clone(),
            current_dir: self.current_dir.clone(),
            wait_timeout: self.wait_timeout.clone(),
        }
    }
}

/// Errors returned from Helm invocations.
#[derive(Debug, Error)]
pub enum HelmError {
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} exited with status {status:?}\nstderr:\n{stderr}\nstdout:\n{stdout}")]
    Failed {
        command: String,
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
}

/// Uninstall the release and namespace resources.
pub async fn uninstall_release(release: &str, namespace: &str) -> Result<(), HelmError> {
    let mut cmd = Command::new("helm");
    cmd.arg("uninstall")
        .arg(release)
        .arg("--namespace")
        .arg(namespace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    info!(release, namespace, "issuing helm uninstall");
    run_helm_command(cmd, &format!("helm uninstall {release}")).await?;
    info!(release, namespace, "helm uninstall completed successfully");
    Ok(())
}

pub async fn install_release(spec: &HelmInstallSpec) -> Result<Output, HelmError> {
    let command = format!("helm install {}", spec.release);
    let output = run_helm_command(build_install_command(spec), &command).await?;
    maybe_log_install_output(&command, &output);
    Ok(output)
}

fn build_install_command(spec: &HelmInstallSpec) -> Command {
    let mut cmd = Command::new("helm");
    cmd.arg("install")
        .arg(&spec.release)
        .arg(&spec.chart_path)
        .arg("--namespace")
        .arg(&spec.namespace)
        .arg("--create-namespace")
        .arg("--wait")
        .arg("--timeout")
        .arg(&spec.wait_timeout);

    for value in &spec.set_values {
        cmd.arg("--set")
            .arg(format!("{}={}", value.key, value.value));
    }

    for values_file in &spec.values_files {
        cmd.arg("-f").arg(values_file);
    }

    for file in &spec.set_files {
        cmd.arg("--set-file")
            .arg(format!("{}={}", file.key, file.path.display()));
    }

    if let Some(current_dir) = &spec.current_dir {
        cmd.current_dir(current_dir);
    }

    cmd
}

fn maybe_log_install_output(command: &str, output: &Output) {
    if std::env::var("K8S_RUNNER_DEBUG").is_err() {
        return;
    }

    debug!(
        command,
        stdout = %String::from_utf8_lossy(&output.stdout),
        "helm install stdout"
    );
    debug!(
        command,
        stderr = %String::from_utf8_lossy(&output.stderr),
        "helm install stderr"
    );
}

async fn run_helm_command(mut cmd: Command, command: &str) -> Result<Output, HelmError> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = cmd.output().await.map_err(|source| HelmError::Spawn {
        command: command.to_owned(),
        source,
    })?;

    if output.status.success() {
        Ok(output)
    } else {
        Err(HelmError::Failed {
            command: command.to_owned(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
