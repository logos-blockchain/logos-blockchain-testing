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
    let runtime_dockerfile = repo_root.join("testing-framework/assets/stack/Dockerfile.runtime");

    tracing::info!(
        image,
        "building compose test image via scripts/build_test_image.sh"
    );

    let mut cmd = Command::new("bash");
    cmd.arg(repo_root.join("scripts/build_test_image.sh"))
        .arg("--tag")
        .arg(image)
        .arg("--dockerfile")
        .arg(runtime_dockerfile)
        // Make the build self-contained (don't require a local bundle tar).
        .arg("--no-restore");

    if let Some(build_platform) = select_build_platform(platform)? {
        cmd.env("DOCKER_DEFAULT_PLATFORM", build_platform);
    }

    if let Some(circuits_platform) = env::var("COMPOSE_CIRCUITS_PLATFORM")
        .ok()
        .filter(|value| !value.is_empty())
    {
        cmd.arg("--circuits-platform").arg(circuits_platform);
    }

    if let Some(value) = env::var("CIRCUITS_OVERRIDE")
        .ok()
        .filter(|val| !val.is_empty())
    {
        cmd.arg("--circuits-override").arg(value);
    }

    cmd.current_dir(&repo_root);

    let status = timeout(
        testing_framework_core::adjust_timeout(IMAGE_BUILD_TIMEOUT),
        cmd.status(),
    )
    .await
    .map_err(|_| {
        warn!(
            image,
            timeout = ?IMAGE_BUILD_TIMEOUT,
            "test image build timed out"
        );
        ComposeRunnerError::Compose(ComposeCommandError::Timeout {
            command: String::from("scripts/build_test_image.sh"),
            timeout: testing_framework_core::adjust_timeout(IMAGE_BUILD_TIMEOUT),
        })
    })?;

    match status {
        Ok(code) if code.success() => {
            info!(image, platform = ?platform, "test image build completed");
            Ok(())
        }
        Ok(code) => {
            warn!(image, status = ?code, "test image build failed");
            Err(ComposeRunnerError::Compose(ComposeCommandError::Failed {
                command: String::from("scripts/build_test_image.sh"),
                status: code,
            }))
        }
        Err(err) => {
            warn!(image, error = ?err, "test image build spawn failed");
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
