use std::{io, process::Stdio};

use thiserror::Error;
use tokio::process::Command;
use tracing::{debug, info};

use crate::infrastructure::assets::{RunnerAssets, cfgsync_port_value, workspace_root};

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

/// Install the Helm release for the provided topology counts.
pub async fn install_release(
    assets: &RunnerAssets,
    release: &str,
    namespace: &str,
    nodes: usize,
) -> Result<(), HelmError> {
    info!(
        release,
        namespace,
        nodes,
        image = %assets.image,
        cfgsync_port = cfgsync_port_value(),
        values = %assets.values_file.display(),
        "installing helm release"
    );

    let command = format!("helm install {release}");
    let cmd = build_install_command(assets, release, namespace, nodes, &command);
    let output = run_helm_command(cmd, &command).await?;

    maybe_log_install_output(&command, &output);

    info!(release, namespace, "helm install completed");
    Ok(())
}

fn build_install_command(
    assets: &RunnerAssets,
    release: &str,
    namespace: &str,
    nodes: usize,
    command: &str,
) -> Command {
    let mut cmd = Command::new("helm");
    cmd.arg("install")
        .arg(release)
        .arg(&assets.chart_path)
        .arg("--namespace")
        .arg(namespace)
        .arg("--create-namespace")
        .arg("--wait")
        .arg("--timeout")
        .arg("5m")
        .arg("--set")
        .arg(format!("image={}", assets.image))
        .arg("--set")
        .arg(format!("nodes.count={nodes}"))
        .arg("--set")
        .arg(format!("cfgsync.port={}", cfgsync_port_value()))
        .arg("-f")
        .arg(&assets.values_file)
        .arg("--set-file")
        .arg(format!("cfgsync.config={}", assets.cfgsync_file.display()))
        .arg("--set-file")
        .arg(format!(
            "scripts.runCfgsyncSh={}",
            assets.run_cfgsync_script.display()
        ))
        .arg("--set-file")
        .arg(format!(
            "scripts.runNomosNodeSh={}",
            assets.run_nomos_node_script.display()
        ))
        .arg("--set-file")
        .arg(format!(
            "scripts.runNomosSh={}",
            assets.run_nomos_script.display()
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Ok(root) = workspace_root() {
        cmd.current_dir(root);
    }

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    debug!(command, "prepared helm install command");
    cmd
}

fn maybe_log_install_output(command: &str, output: &std::process::Output) {
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

async fn run_helm_command(
    mut cmd: Command,
    command: &str,
) -> Result<std::process::Output, HelmError> {
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
