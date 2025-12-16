use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use tera::Context as TeraContext;
use tracing::{debug, info};

use crate::descriptor::ComposeDescriptor;

const TEMPLATE_RELATIVE_PATH: &str =
    "testing-framework/runners/compose/assets/docker-compose.yml.tera";

/// Errors when templating docker-compose files.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("failed to resolve repository root for compose template: {source}")]
    RepositoryRoot {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to read compose template at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialise compose descriptor for templating: {source}")]
    Serialize {
        #[source]
        source: tera::Error,
    },
    #[error("failed to render compose template at {path}: {source}")]
    Render {
        path: PathBuf,
        #[source]
        source: tera::Error,
    },
    #[error("failed to write compose file at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Render and write the compose file to disk.
pub fn write_compose_file(
    descriptor: &ComposeDescriptor,
    compose_path: &Path,
) -> Result<(), TemplateError> {
    info!(file = %compose_path.display(), "writing compose file");
    TemplateSource::load()?.write(descriptor, compose_path)
}

struct TemplateSource {
    path: PathBuf,
    contents: String,
}

impl TemplateSource {
    fn load() -> Result<Self, TemplateError> {
        let repo_root =
            repository_root().map_err(|source| TemplateError::RepositoryRoot { source })?;
        let path = repo_root.join(TEMPLATE_RELATIVE_PATH);
        let contents = fs::read_to_string(&path).map_err(|source| TemplateError::Read {
            path: path.clone(),
            source,
        })?;

        Ok(Self { path, contents })
    }

    fn render(&self, descriptor: &ComposeDescriptor) -> Result<String, TemplateError> {
        debug!(template = %self.path.display(), "rendering compose template");
        let context = TeraContext::from_serialize(descriptor)
            .map_err(|source| TemplateError::Serialize { source })?;

        tera::Tera::one_off(&self.contents, &context, false).map_err(|source| {
            TemplateError::Render {
                path: self.path.clone(),
                source,
            }
        })
    }

    fn write(&self, descriptor: &ComposeDescriptor, output: &Path) -> Result<(), TemplateError> {
        let rendered = self.render(descriptor)?;
        fs::write(output, rendered).map_err(|source| TemplateError::Write {
            path: output.to_path_buf(),
            source,
        })
    }
}

/// Resolve the repository root, respecting `CARGO_WORKSPACE_DIR` override.
pub fn repository_root() -> anyhow::Result<PathBuf> {
    env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .map(PathBuf::from)
                .context("resolving repository root from manifest dir")
        })
}
