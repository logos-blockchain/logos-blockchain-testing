use std::{
    io,
    process::{Output, Stdio},
};

use thiserror::Error;
use tokio::process::Command;
use tracing::info;

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
