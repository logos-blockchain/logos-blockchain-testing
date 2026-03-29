use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use tempfile::TempDir;
use thiserror::Error;

const DEFAULT_RUNNER_HELM_CHART_DIR: &str = "testing-framework/deployers/k8s/assets/helm/tf-runner";

fn crate_runner_chart_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/helm/tf-runner")
}

pub fn resolve_workspace_root<F>(
    manifest_dir: &Path,
    env_override_var: &str,
    is_workspace_root: F,
) -> Result<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    if let Ok(var) = env::var(env_override_var) {
        return Ok(PathBuf::from(var));
    }

    let candidate_roots = [
        manifest_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent),
        manifest_dir.parent().and_then(Path::parent),
    ];

    for candidate in candidate_roots.iter().flatten() {
        if is_workspace_root(candidate) {
            return Ok(candidate.to_path_buf());
        }
    }

    Err(anyhow!(
        "resolving workspace root from manifest dir: {manifest_dir:?}"
    ))
}

#[must_use]
pub fn bundled_runner_chart_path(workspace_root: &Path) -> PathBuf {
    let workspace_path = workspace_root.join(DEFAULT_RUNNER_HELM_CHART_DIR);
    if workspace_path.exists() {
        workspace_path
    } else {
        crate_runner_chart_path()
    }
}

#[must_use]
pub fn resolve_optional_relative_dir(workspace_root: &Path, env_key: &str) -> Option<PathBuf> {
    env::var(env_key).ok().map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            workspace_root.join(path)
        }
    })
}

#[derive(Debug, Error)]
pub enum RequiredPathError {
    #[error("missing required path at {}", path.display())]
    MissingPath { path: PathBuf },
    #[error("failed to read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn require_existing_paths(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<PathBuf>, RequiredPathError> {
    let mut collected = Vec::new();

    for path in paths {
        match path.try_exists() {
            Ok(true) => collected.push(path),
            Ok(false) => return Err(RequiredPathError::MissingPath { path }),
            Err(source) => return Err(RequiredPathError::Io { path, source }),
        }
    }

    Ok(collected)
}

pub fn create_temp_workspace(prefix: &str) -> Result<TempDir, io::Error> {
    tempfile::Builder::new().prefix(prefix).tempdir()
}

pub fn write_temp_file(
    dir: &Path,
    name: &str,
    contents: impl AsRef<[u8]>,
) -> Result<PathBuf, io::Error> {
    let path = dir.join(name);
    fs::write(&path, contents)?;
    Ok(path)
}
