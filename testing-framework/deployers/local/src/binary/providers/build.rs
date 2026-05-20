//! Command build provider.
//!
//! This provider delegates binary preparation to a command supplied by the test
//! or app integration. The command may run Cargo, fetch from an internal cache,
//! invoke a project-specific build script, or do anything else needed to
//! produce the expected executable path.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use tracing::info;

use crate::binary::{
    BinaryProvider, BinaryProviderError, BuildBinaryProvider, lock::BinaryProviderLock,
    optional_path_display,
};

impl BinaryProvider for BuildBinaryProvider {
    fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError> {
        let output_path = self.output_path();
        let _lock = BinaryProviderLock::acquire(&self.lock_path())?;

        self.run_build()?;
        self.ensure_output_exists(&output_path)?;

        Ok(Some(output_path))
    }

    fn display(&self) -> String {
        "build".to_owned()
    }

    fn cache_key(&self) -> String {
        format!(
            "build:{}:{}:{}",
            self.command.display(),
            self.output_path.display(),
            optional_path_display(&self.working_dir)
        )
    }
}

impl BuildBinaryProvider {
    fn run_build(&self) -> Result<(), BinaryProviderError> {
        info!(
            command = self.command.display(),
            workspace = %self.workspace_dir().display(),
            "building binary"
        );

        let status = self
            .command()
            .status()
            .map_err(|source| BinaryProviderError::Io {
                path: self.workspace_dir(),
                source,
            })?;

        if !status.success() {
            return Err(BinaryProviderError::BuildFailed {
                status: status.to_string(),
            });
        }

        Ok(())
    }

    fn ensure_output_exists(&self, output_path: &Path) -> Result<(), BinaryProviderError> {
        if output_path.is_file() {
            return Ok(());
        }

        Err(BinaryProviderError::MissingBuildOutput {
            path: output_path.to_owned(),
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.command.program);
        command
            .current_dir(self.workspace_dir())
            .args(&self.command.args);
        command
    }

    fn output_path(&self) -> PathBuf {
        if self.output_path.is_absolute() {
            return self.output_path.clone();
        }

        self.workspace_dir().join(&self.output_path)
    }

    fn lock_path(&self) -> PathBuf {
        let lock_file_name = self
            .output_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("binary");

        self.lock_dir
            .clone()
            .unwrap_or_else(|| self.workspace_dir().join(".tf-binaries"))
            .join(format!("{lock_file_name}.lock"))
    }

    fn workspace_dir(&self) -> PathBuf {
        self.working_dir
            .clone()
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}
