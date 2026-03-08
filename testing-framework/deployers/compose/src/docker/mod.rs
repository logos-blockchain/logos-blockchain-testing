pub mod attached;
pub mod commands;
pub mod control;
pub mod platform;
pub mod workspace;

use std::{
    env, io,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

use testing_framework_core::adjust_timeout;
use tokio::{process::Command, time::timeout};
use tracing::{debug, info, warn};

use crate::{
    docker::commands::ComposeCommandError, errors::ComposeRunnerError,
    infrastructure::template::repository_root,
};

const IMAGE_BUILD_TIMEOUT: Duration = Duration::from_secs(600);
const DOCKER_INFO_TIMEOUT: Duration = Duration::from_secs(15);
const IMAGE_INSPECT_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const DEFAULT_ASSETS_STACK_DIR: &str = "logos/infra/assets/stack";

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
    platform: Option<&str>,
) -> Result<(), ComposeRunnerError> {
    if docker_image_exists(image).await? {
        debug!(image, "docker image already present");
        return Ok(());
    }

    if !is_local_test_image(image) {
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

/// Build the local testnet image with optional platform override.
pub async fn build_local_image(
    image: &str,
    platform: Option<&str>,
) -> Result<(), ComposeRunnerError> {
    let repo_root =
        repository_root().map_err(|source| ComposeRunnerError::ImageBuild { source })?;
    info!(
        image,
        "building compose test image via scripts/build/build_test_image.sh"
    );
    let mut cmd = build_local_image_command(&repo_root, image, platform)?;

    match run_build_command_with_timeout(image, &mut cmd).await? {
        Ok(code) if code.success() => {
            info!(image, platform = ?platform, "test image build completed");
            Ok(())
        }

        Ok(code) => {
            warn!(image, status = ?code, "test image build failed");
            Err(ComposeRunnerError::Compose(ComposeCommandError::Failed {
                command: String::from("scripts/build/build_test_image.sh"),
                status: code,
            }))
        }

        Err(err) => {
            warn!(image, error = ?err, "test image build spawn failed");
            Err(ComposeRunnerError::ImageBuild { source: err.into() })
        }
    }
}

fn build_local_image_command(
    repo_root: &Path,
    image: &str,
    platform: Option<&str>,
) -> Result<Command, ComposeRunnerError> {
    let runtime_dockerfile = stack_assets_root(repo_root).join("Dockerfile.runtime");
    let mut cmd = Command::new("bash");

    cmd.arg(repo_root.join("scripts/build/build_test_image.sh"))
        .arg("--tag")
        .arg(image)
        .arg("--dockerfile")
        .arg(runtime_dockerfile)
        // Make the build self-contained (don't require a local bundle tar).
        .arg("--no-restore")
        .current_dir(repo_root);

    if let Some(build_platform) = select_build_platform(platform) {
        cmd.env("DOCKER_DEFAULT_PLATFORM", build_platform);
    }

    apply_optional_circuits_flags(&mut cmd);

    Ok(cmd)
}

async fn run_build_command_with_timeout(
    image: &str,
    cmd: &mut Command,
) -> Result<Result<ExitStatus, io::Error>, ComposeRunnerError> {
    let timeout_duration = adjust_timeout(IMAGE_BUILD_TIMEOUT);

    timeout(timeout_duration, cmd.status()).await.map_err(|_| {
        warn!(
            image,
            timeout = ?IMAGE_BUILD_TIMEOUT,
            "test image build timed out"
        );
        ComposeRunnerError::Compose(ComposeCommandError::Timeout {
            command: String::from("scripts/build/build_test_image.sh"),
            timeout: timeout_duration,
        })
    })
}

fn select_build_platform(platform: Option<&str>) -> Option<String> {
    platform.map(String::from).or_else(|| {
        let host_arch = env::consts::ARCH;
        match host_arch {
            "aarch64" | "arm64" => Some(String::from("linux/arm64")),
            "x86_64" => Some(String::from("linux/amd64")),
            _ => None,
        }
    })
}

fn apply_optional_circuits_flags(cmd: &mut Command) {
    if let Some(circuits_platform) = nonempty_env("COMPOSE_CIRCUITS_PLATFORM") {
        cmd.arg("--circuits-platform").arg(circuits_platform);
    }

    if let Some(value) = nonempty_env("CIRCUITS_OVERRIDE") {
        cmd.arg("--circuits-override").arg(value);
    }
}

fn nonempty_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.is_empty())
}

fn stack_assets_root(repo_root: &Path) -> PathBuf {
    if let Some(override_dir) = assets_override_dir(repo_root)
        && override_dir.exists()
    {
        return override_dir;
    }

    repo_root.join(DEFAULT_ASSETS_STACK_DIR)
}

fn is_local_test_image(image: &str) -> bool {
    image == "logos-blockchain-testing:local"
}

fn assets_override_dir(repo_root: &Path) -> Option<PathBuf> {
    env::var("REL_ASSETS_STACK_DIR").ok().map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        }
    })
}
