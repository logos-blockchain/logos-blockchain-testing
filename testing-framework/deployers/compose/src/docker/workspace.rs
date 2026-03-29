use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use tempfile::TempDir;
use tracing::{debug, info};

use super::DEFAULT_ASSETS_STACK_DIR;

/// Copy the repository stack assets into a scenario-specific temp dir.
#[derive(Debug)]
pub struct ComposeWorkspace {
    root: TempDir,
}

impl ComposeWorkspace {
    /// Clone the stack assets into a temporary directory.
    pub fn create() -> Result<Self> {
        let repo_root = resolve_repo_root()?;
        let temp = create_temp_workspace()?;
        let stack_source = stack_assets_root(&repo_root);
        ensure_stack_source_exists(&stack_source)?;

        debug!(
            repo_root = %repo_root.display(),
            stack_source = %stack_source.display(),
            "copying stack assets into temporary workspace"
        );
        copy_stack_assets(&repo_root, &stack_source, temp.path())?;

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

fn resolve_repo_root() -> Result<PathBuf> {
    env::var("REPO_ROOT_OVERRIDE_DIR")
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
        .context("locating repository root")
}

fn create_temp_workspace() -> Result<TempDir> {
    tempfile::Builder::new()
        .prefix("compose-stack-")
        .tempdir()
        .context("creating testnet temp dir")
}

fn ensure_stack_source_exists(stack_source: &Path) -> Result<()> {
    if !stack_source.exists() {
        anyhow::bail!(
            "stack assets directory not found at {}",
            stack_source.display()
        );
    }
    Ok(())
}

fn copy_stack_assets(repo_root: &Path, stack_source: &Path, target_root: &Path) -> Result<()> {
    copy_dir_recursive(&stack_source, &target_root.join("stack"))?;

    let scripts_source = stack_scripts_root(repo_root, stack_source);
    if scripts_source.exists() {
        copy_dir_recursive(&scripts_source, &target_root.join("stack/scripts"))?;
    }

    Ok(())
}

fn stack_assets_root(repo_root: &Path) -> PathBuf {
    if let Some(override_dir) = assets_override_dir(repo_root)
        && override_dir.exists()
    {
        return override_dir;
    }

    let default_stack = repo_root.join(DEFAULT_ASSETS_STACK_DIR);
    if default_stack.exists() {
        return default_stack;
    }

    current_dir_runtime_assets().unwrap_or(default_stack)
}

fn stack_scripts_root(repo_root: &Path, stack_source: &Path) -> PathBuf {
    let scripts = stack_source.join("scripts");
    if scripts.exists() {
        return scripts;
    }

    repo_root.join(DEFAULT_ASSETS_STACK_DIR).join("scripts")
}

fn assets_override_dir(repo_root: &Path) -> Option<PathBuf> {
    env::var("REL_ASSETS_STACK_DIR")
        .ok()
        .map(PathBuf::from)
        .map(|path| resolve_workspace_relative_path(repo_root, path))
}

fn resolve_workspace_relative_path(repo_root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }

    repo_root.join(path)
}

fn current_dir_runtime_assets() -> Option<PathBuf> {
    let candidate = env::current_dir()
        .ok()?
        .join("tests/testing_framework/assets/runtime");

    candidate.exists().then_some(candidate)
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
        } else {
            fs::copy(entry.path(), &dest).with_context(|| {
                format!("copying {} -> {}", entry.path().display(), dest.display())
            })?;
        }
    }

    Ok(())
}
