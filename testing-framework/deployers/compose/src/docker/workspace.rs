use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use tempfile::TempDir;
use testing_framework_config::constants::DEFAULT_ASSETS_STACK_DIR;
use tracing::{debug, info};

/// Copy the repository stack assets into a scenario-specific temp dir.
#[derive(Debug)]
pub struct ComposeWorkspace {
    root: TempDir,
}

impl ComposeWorkspace {
    /// Clone the stack assets into a temporary directory.
    pub fn create() -> Result<Self> {
        let repo_root = env::var("REPO_ROOT_OVERRIDE_DIR")
            .or_else(|_| env::var("CARGO_WORKSPACE_DIR"))
            .map(PathBuf::from)
            .or_else(|_| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
                    .context("resolving workspace root from manifest dir")
            })
            .context("locating repository root")?;
        let temp = tempfile::Builder::new()
            .prefix("nomos-testnet-")
            .tempdir()
            .context("creating testnet temp dir")?;
        let stack_source = stack_assets_root(&repo_root);
        if !stack_source.exists() {
            anyhow::bail!(
                "stack assets directory not found at {}",
                stack_source.display()
            );
        }
        debug!(
            repo_root = %repo_root.display(),
            stack_source = %stack_source.display(),
            "copying stack assets into temporary workspace"
        );
        copy_dir_recursive(&stack_source, &temp.path().join("stack"))?;
        let scripts_source = stack_scripts_root(&repo_root);
        if scripts_source.exists() {
            copy_dir_recursive(&scripts_source, &temp.path().join("stack/scripts"))?;
        }

        info!(root = %temp.path().display(), "compose workspace created");
        Ok(Self { root: temp })
    }

    #[must_use]
    /// Root of the temporary workspace on disk.
    pub fn root_path(&self) -> &Path {
        self.root.path()
    }

    #[must_use]
    /// Path to the copied assets directory.
    pub fn stack_dir(&self) -> PathBuf {
        self.root.path().join("stack")
    }

    #[must_use]
    /// Consume the workspace and return the underlying temp directory.
    pub fn into_inner(self) -> TempDir {
        self.root
    }
}

fn stack_assets_root(repo_root: &Path) -> PathBuf {
    let new_layout = if let Some(rel_stack_dir) = env::var("REL_ASSETS_STACK_DIR").ok() {
        repo_root.join(rel_stack_dir)
    } else {
        repo_root.join(DEFAULT_ASSETS_STACK_DIR)
    };
    if new_layout.exists() {
        new_layout
    } else {
        repo_root.join("testnet")
    }
}

fn stack_scripts_root(repo_root: &Path) -> PathBuf {
    let new_layout = repo_root.join(DEFAULT_ASSETS_STACK_DIR).join("scripts");
    if new_layout.exists() {
        new_layout
    } else {
        repo_root.join("testnet/scripts")
    }
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("creating target dir {}", target.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("reading {}", source.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else if !file_type.is_dir() {
            fs::copy(entry.path(), &dest).with_context(|| {
                format!("copying {} -> {}", entry.path().display(), dest.display())
            })?;
        }
    }
    Ok(())
}
