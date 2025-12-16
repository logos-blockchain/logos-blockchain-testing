pub mod commands;
pub mod control;
pub mod platform;
pub mod workspace;

use std::{env, process::Stdio, time::Duration};

use tokio::{process::Command, time::timeout};
use tracing::{debug, info, warn};

use crate::{
    docker::commands::ComposeCommandError, errors::ComposeRunnerError,
    infrastructure::template::repository_root,
};

const IMAGE_BUILD_TIMEOUT: Duration = Duration::from_secs(600);
const DOCKER_INFO_TIMEOUT: Duration = Duration::from_secs(15);
const IMAGE_INSPECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Checks that `docker info` succeeds within a timeout.
pub async fn ensure_docker_available() -> Result<(), ComposeRunnerError> {
    let mut command = Command::new("docker");
    command
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let available = timeout(
        testing_framework_core::adjust_timeout(DOCKER_INFO_TIMEOUT),
        command.status(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .map(|status| status.success())
    .unwrap_or(false);

    if available {
        debug!("docker info succeeded");
        Ok(())
    } else {
        warn!("docker info failed or timed out; compose runner unavailable");
        Err(ComposeRunnerError::DockerUnavailable)
    }
}

/// Ensure the configured compose image exists, building a local one if needed.
pub async fn ensure_compose_image() -> Result<(), ComposeRunnerError> {
    let (image, platform) = crate::docker::platform::resolve_image();
    info!(image, platform = ?platform, "ensuring compose image is present");
    ensure_image_present(&image, platform.as_deref()).await
}

/// Verify an image exists locally, optionally building it for the default tag.
pub async fn ensure_image_present(
    image: &str,
    platform: Option<&str>,
) -> Result<(), ComposeRunnerError> {
    if docker_image_exists(image).await? {
        debug!(image, "docker image already present");
        return Ok(());
    }

    if image != "logos-blockchain-testing:local" {
        return Err(ComposeRunnerError::MissingImage {
            image: image.to_owned(),
        });
    }

    build_local_image(image, platform).await
}

/// Returns true when `docker image inspect` succeeds for the image.
pub async fn docker_image_exists(image: &str) -> Result<bool, ComposeRunnerError> {
    let mut cmd = Command::new("docker");
    cmd.arg("image")
        .arg("inspect")
        .arg(image)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match timeout(
        testing_framework_core::adjust_timeout(IMAGE_INSPECT_TIMEOUT),
        cmd.status(),
    )
    .await
    {
        Ok(Ok(status)) => Ok(status.success()),
        Ok(Err(source)) => Err(ComposeRunnerError::Compose(ComposeCommandError::Spawn {
            command: format!("docker image inspect {image}"),
            source,
        })),
        Err(_) => Err(ComposeRunnerError::Compose(ComposeCommandError::Timeout {
            command: format!("docker image inspect {image}"),
            timeout: testing_framework_core::adjust_timeout(IMAGE_INSPECT_TIMEOUT),
        })),
    }
}

/// Build the local testnet image with optional platform override.
pub async fn build_local_image(
    image: &str,
    platform: Option<&str>,
) -> Result<(), ComposeRunnerError> {
    let repo_root =
        repository_root().map_err(|source| ComposeRunnerError::ImageBuild { source })?;
    let dockerfile = repo_root.join("../../../docker/runner.Dockerfile");

    tracing::info!(image, "building compose runner docker image");

    let mut cmd = Command::new("docker");
    cmd.arg("build");

    if let Some(build_platform) = select_build_platform(platform)? {
        cmd.arg("--platform").arg(&build_platform);
    }

    let circuits_platform = env::var("COMPOSE_CIRCUITS_PLATFORM")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| String::from("linux-x86_64"));

    cmd.arg("--build-arg")
        .arg(format!("NOMOS_CIRCUITS_PLATFORM={circuits_platform}"));

    if let Some(value) = env::var("CIRCUITS_OVERRIDE")
        .ok()
        .filter(|val| !val.is_empty())
    {
        cmd.arg("--build-arg")
            .arg(format!("CIRCUITS_OVERRIDE={value}"));
    }

    let node_rev = std::env::var("NOMOS_NODE_REV")
        .unwrap_or_else(|_| String::from("d2dd5a5084e1daef4032562c77d41de5e4d495f8"));
    cmd.arg("--build-arg")
        .arg(format!("NOMOS_NODE_REV={node_rev}"));

    if let Some(value) = env::var("NOMOS_CIRCUITS_VERSION")
        .ok()
        .filter(|val| !val.is_empty())
    {
        cmd.arg("--build-arg")
            .arg(format!("NOMOS_CIRCUITS_VERSION={value}"));
    }

    if env::var("NOMOS_CIRCUITS_REBUILD_RAPIDSNARK").is_ok() {
        cmd.arg("--build-arg").arg("RAPIDSNARK_REBUILD=1");
    }

    cmd.arg("-t")
        .arg(image)
        .arg("-f")
        .arg(dockerfile)
        .arg(&repo_root);

    cmd.current_dir(&repo_root);

    let status = timeout(
        testing_framework_core::adjust_timeout(IMAGE_BUILD_TIMEOUT),
        cmd.status(),
    )
    .await
    .map_err(|_| {
        warn!(image, timeout = ?IMAGE_BUILD_TIMEOUT, "docker build timed out");
        ComposeRunnerError::Compose(ComposeCommandError::Timeout {
            command: String::from("docker build"),
            timeout: testing_framework_core::adjust_timeout(IMAGE_BUILD_TIMEOUT),
        })
    })?;

    match status {
        Ok(code) if code.success() => {
            info!(image, platform = ?platform, "docker build completed");
            Ok(())
        }
        Ok(code) => {
            warn!(image, status = ?code, "docker build failed");
            Err(ComposeRunnerError::Compose(ComposeCommandError::Failed {
                command: String::from("docker build"),
                status: code,
            }))
        }
        Err(err) => {
            warn!(image, error = ?err, "docker build spawn failed");
            Err(ComposeRunnerError::ImageBuild { source: err.into() })
        }
    }
}

fn select_build_platform(platform: Option<&str>) -> Result<Option<String>, ComposeRunnerError> {
    Ok(platform.map(String::from).or_else(|| {
        let host_arch = std::env::consts::ARCH;
        match host_arch {
            "aarch64" | "arm64" => Some(String::from("linux/arm64")),
            "x86_64" => Some(String::from("linux/amd64")),
            _ => None,
        }
    }))
}
