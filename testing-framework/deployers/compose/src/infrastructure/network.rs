use std::time::Duration;

use testing_framework_core::adjust_timeout;
use tokio::{process::Command, sync::Mutex};
use tracing::{info, warn};
use uuid::Uuid;

use crate::docker::commands::run_docker_command;

/// Label applied to every shared session network so leaked networks stay
/// identifiable and prunable.
pub(crate) const SESSION_NETWORK_LABEL: &str = "testing-framework=session";

const NETWORK_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// The external Docker network shared by all participants of one provisioner.
///
/// The network is created lazily by the first deployment and removed by the
/// last participant to leave; creation and removal serialize on the same lock
/// so the liveness check backing a removal cannot race a concurrent first
/// deployment.
pub(crate) struct SharedNetwork {
    name: String,
    created: Mutex<bool>,
}

impl SharedNetwork {
    pub(crate) fn new() -> Self {
        Self {
            name: format!("tf-session-{}", Uuid::new_v4()),
            created: Mutex::new(false),
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Creates the labeled session network unless it already exists.
    pub(crate) async fn ensure_created(&self) -> Result<(), crate::errors::ComposeRunnerError> {
        let mut created = self.created.lock().await;
        if *created {
            return Ok(());
        }

        let mut command = Command::new("docker");
        command.args([
            "network",
            "create",
            "--label",
            SESSION_NETWORK_LABEL,
            &self.name,
        ]);
        run_docker_command(
            command,
            adjust_timeout(NETWORK_COMMAND_TIMEOUT),
            "docker network create",
        )
        .await?;

        info!(network = %self.name, "created shared compose session network");
        *created = true;
        Ok(())
    }

    /// Removes the session network unless it was never created or `in_use`
    /// reports it is still needed; removal failures are logged and swallowed.
    pub(crate) async fn remove_if_unused(&self, in_use: impl FnOnce() -> bool) {
        let mut created = self.created.lock().await;
        if !*created || in_use() {
            return;
        }

        let mut command = Command::new("docker");
        command.args(["network", "rm", &self.name]);
        match run_docker_command(
            command,
            adjust_timeout(NETWORK_COMMAND_TIMEOUT),
            "docker network rm",
        )
        .await
        {
            Ok(()) => {
                info!(network = %self.name, "removed shared compose session network");
                *created = false;
            }
            Err(source) => warn!(
                network = %self.name,
                error = %source,
                "failed to remove shared compose session network; remove it manually"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SharedNetwork;

    #[tokio::test]
    async fn removal_is_a_no_op_before_creation() {
        let network = SharedNetwork::new();

        network.remove_if_unused(|| false).await;
        network
            .remove_if_unused(|| unreachable!("liveness must not be consulted before creation"))
            .await;
    }

    #[test]
    fn network_names_are_unique_per_provisioner() {
        let first = SharedNetwork::new();
        let second = SharedNetwork::new();

        assert_ne!(first.name(), second.name());
        assert!(first.name().starts_with("tf-session-"));
    }
}
