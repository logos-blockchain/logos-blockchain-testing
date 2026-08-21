use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, anyhow};
use testing_framework_core::{adjust_timeout, scenario::DynError};
use tokio::{
    process::Command,
    sync::{OwnedRwLockWriteGuard, RwLock},
};
use uuid::Uuid;

use crate::{
    docker::{
        commands::{
            ComposeCommandError, compose_create, compose_down, compose_rm_services,
            compose_start_service, compose_stop_service, compose_up, compose_up_service,
            dump_compose_logs,
        },
        control::restart_compose_service,
        workspace::ComposeWorkspace,
    },
    errors::ComposeRunnerError,
    infrastructure::ports::resolve_service_port_with,
};

const COMPOSE_STATUS_TIMEOUT: Duration = Duration::from_secs(120);

/// A managed Docker Compose project shared by all Compose-facing adapters.
///
/// This is the backend runtime boundary: it owns project identity and
/// serializes descriptor mutations against service lifecycle operations.
/// Uniform scenarios and heterogeneous application stacks translate their
/// inputs independently, then operate through this same project runtime.
#[derive(Clone, Debug)]
pub(crate) struct ComposeProject {
    compose_file: PathBuf,
    name: String,
    root: PathBuf,
    access: Arc<RwLock<()>>,
}

impl ComposeProject {
    pub(crate) fn for_workspace(workspace: &ComposeWorkspace, name: &str) -> Self {
        let root = workspace.root_path().to_path_buf();
        let prefix = normalized_project_prefix(name);

        Self::new(
            root.join("compose.generated.yml"),
            format!("{prefix}-{}", Uuid::new_v4()),
            root,
        )
    }

    fn new(compose_file: PathBuf, name: impl Into<String>, root: PathBuf) -> Self {
        Self {
            compose_file,
            name: name.into(),
            root,
            access: Arc::new(RwLock::new(())),
        }
    }

    pub(crate) fn compose_file(&self) -> &Path {
        &self.compose_file
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Exclude lifecycle operations while an adapter replaces the project
    /// descriptor and applies an incremental change.
    pub(crate) async fn lock_mutation(&self) -> OwnedRwLockWriteGuard<()> {
        Arc::clone(&self.access).write_owned().await
    }

    pub(crate) async fn create(&self) -> Result<(), ComposeCommandError> {
        let _access = self.access.read().await;
        compose_create(&self.compose_file, &self.name, &self.root).await
    }

    pub(crate) async fn up(&self) -> Result<(), ComposeCommandError> {
        let _access = self.access.read().await;
        self.up_unlocked().await
    }

    pub(crate) async fn up_unlocked(&self) -> Result<(), ComposeCommandError> {
        compose_up(&self.compose_file, &self.name, &self.root).await
    }

    pub(crate) async fn up_service_unlocked(
        &self,
        service: &str,
    ) -> Result<(), ComposeCommandError> {
        compose_up_service(&self.compose_file, &self.name, &self.root, service).await
    }

    pub(crate) async fn start_service(&self, service: &str) -> Result<(), ComposeCommandError> {
        let _access = self.access.read().await;
        compose_start_service(&self.compose_file, &self.name, &self.root, service).await
    }

    pub(crate) async fn stop_service(&self, service: &str) -> Result<(), ComposeCommandError> {
        let _access = self.access.read().await;
        compose_stop_service(&self.compose_file, &self.name, &self.root, service).await
    }

    pub(crate) async fn restart_service(&self, service: &str) -> Result<(), ComposeRunnerError> {
        let _access = self.access.read().await;
        restart_compose_service(&self.compose_file, &self.name, service).await
    }

    pub(crate) async fn remove_services_unlocked(
        &self,
        services: &[String],
    ) -> Result<(), ComposeCommandError> {
        compose_rm_services(&self.compose_file, &self.name, &self.root, services).await
    }

    pub(crate) async fn down_unlocked(&self) -> Result<(), ComposeCommandError> {
        compose_down(&self.compose_file, &self.name, &self.root).await
    }

    /// Tears down by project labels using a minimal valid Compose file. This
    /// remains usable when the generated descriptor was left unreadable by a
    /// failed write or restore.
    pub(crate) async fn down_with_fallback_unlocked(&self) -> Result<(), ComposeCommandError> {
        let fallback = write_cleanup_compose_file(&self.root)?;
        compose_down(&fallback, &self.name, &self.root).await
    }

    pub(crate) async fn dump_logs(&self) {
        let _access = self.access.read().await;
        dump_compose_logs(&self.compose_file, &self.name, &self.root).await;
    }

    pub(crate) async fn resolve_service_port(
        &self,
        service: &str,
        container_port: u16,
    ) -> Result<u16, ComposeRunnerError> {
        let _access = self.access.read().await;
        resolve_service_port_with(
            &self.compose_file,
            &self.name,
            &self.root,
            service,
            container_port,
        )
        .await
    }

    pub(crate) async fn service_is_running(&self, service: &str) -> Result<bool, DynError> {
        let _access = self.access.read().await;
        let mut command = Command::new("docker");
        command
            .arg("compose")
            .arg("-f")
            .arg(&self.compose_file)
            .arg("-p")
            .arg(&self.name)
            .args(["ps", "--status", "running", "-q", service])
            .current_dir(&self.root);

        let timeout = adjust_timeout(COMPOSE_STATUS_TIMEOUT);
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| anyhow!("docker compose service status timed out after {timeout:?}"))?
            .with_context(|| format!("checking compose service '{service}' status"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "checking compose service '{}' status failed with {}: {}",
                service,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        Ok(!output.stdout.is_empty())
    }
}

fn write_cleanup_compose_file(root: &Path) -> Result<PathBuf, ComposeCommandError> {
    let fallback = root.join("compose.cleanup.yml");
    fs::write(&fallback, "services: {}\n").map_err(|source| ComposeCommandError::Spawn {
        command: "write fallback compose cleanup file".to_owned(),
        source,
    })?;
    Ok(fallback)
}

fn normalized_project_prefix(name: &str) -> String {
    let normalized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('-');
    if normalized.is_empty() {
        "app-stack".to_owned()
    } else {
        normalized.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::{ComposeProject, normalized_project_prefix, write_cleanup_compose_file};

    fn project() -> ComposeProject {
        ComposeProject::new(
            PathBuf::from("/tmp/compose.yml"),
            "test-project",
            PathBuf::from("/tmp"),
        )
    }

    #[test]
    fn carries_one_compose_project_identity() {
        let project = project();

        assert_eq!(project.compose_file(), PathBuf::from("/tmp/compose.yml"));
        assert_eq!(project.name(), "test-project");
        assert_eq!(project.root(), PathBuf::from("/tmp"));
    }

    #[tokio::test]
    async fn clones_share_the_project_mutation_lock() {
        let project = project();
        let clone = project.clone();
        let guard = project.lock_mutation().await;

        assert!(
            tokio::time::timeout(Duration::from_millis(10), clone.lock_mutation())
                .await
                .is_err()
        );

        drop(guard);
        tokio::time::timeout(Duration::from_secs(1), clone.lock_mutation())
            .await
            .expect("clone should acquire the shared lock after release");
    }

    #[test]
    fn normalizes_project_names_once_for_all_adapters() {
        assert_eq!(normalized_project_prefix("LEZ stack"), "lez-stack");
        assert_eq!(normalized_project_prefix("---"), "app-stack");
    }

    #[test]
    fn cleanup_fallback_uses_an_independent_valid_descriptor() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_cleanup_compose_file(directory.path()).unwrap();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "services: {}\n");
    }
}
