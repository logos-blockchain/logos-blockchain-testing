pub mod attached;
pub mod commands;
pub mod config_server;
pub mod control;
pub mod platform;
pub mod workspace;

use std::{process::Stdio, time::Duration};

use testing_framework_core::adjust_timeout;
use tokio::{process::Command, time::timeout};
use tracing::{debug, warn};

use crate::{docker::commands::ComposeCommandError, errors::ComposeRunnerError};

const DOCKER_INFO_TIMEOUT: Duration = Duration::from_secs(15);
const IMAGE_INSPECT_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const DEFAULT_ASSETS_STACK_DIR: &str = "testing-framework/assets/stack";

/// Checks that `docker info` succeeds within a timeout.
pub async fn ensure_docker_available() -> Result<(), ComposeRunnerError> {
    let mut command = Command::new("docker");
    command
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let available = match timeout(adjust_timeout(DOCKER_INFO_TIMEOUT), command.status()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(_)) | Err(_) => false,
    };

    if available {
        debug!("docker info succeeded");
        Ok(())
    } else {
        warn!("docker info failed or timed out; compose runner unavailable");
        Err(ComposeRunnerError::DockerUnavailable)
    }
}

/// Verify an image exists locally, optionally building it for the default tag.
pub async fn ensure_image_present(
    image: &str,
    _platform: Option<&str>,
) -> Result<(), ComposeRunnerError> {
    if docker_image_exists(image).await? {
        debug!(image, "docker image already present");
        return Ok(());
    }

    Err(ComposeRunnerError::MissingImage {
        image: image.to_owned(),
    })
}

/// Returns true when `docker image inspect` succeeds for the image.
pub async fn docker_image_exists(image: &str) -> Result<bool, ComposeRunnerError> {
    let mut cmd = Command::new("docker");
    cmd.arg("image")
        .arg("inspect")
        .arg(image)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match timeout(adjust_timeout(IMAGE_INSPECT_TIMEOUT), cmd.status()).await {
        Ok(Ok(status)) => Ok(status.success()),
        Ok(Err(source)) => Err(ComposeRunnerError::Compose(ComposeCommandError::Spawn {
            command: format!("docker image inspect {image}"),
            source,
        })),
        Err(_) => Err(ComposeRunnerError::Compose(ComposeCommandError::Timeout {
            command: format!("docker image inspect {image}"),
            timeout: adjust_timeout(IMAGE_INSPECT_TIMEOUT),
        })),
    }
}
