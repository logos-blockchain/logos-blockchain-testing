use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Single file artifact delivered to a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactFile {
    /// Destination path where content should be written.
    pub path: String,
    /// Raw file contents.
    pub content: String,
}

impl ArtifactFile {
    #[must_use]
    pub fn new(path: String, content: String) -> Self {
        Self { path, content }
    }
}

/// Collection of files delivered together for one node.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ArtifactSet {
    pub files: Vec<ArtifactFile>,
}

impl ArtifactSet {
    #[must_use]
    pub fn new(files: Vec<ArtifactFile>) -> Self {
        Self { files }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Validates that no two files target the same output path.
    pub fn ensure_unique_paths(&self) -> Result<(), ArtifactValidationError> {
        let mut seen = std::collections::HashSet::new();

        for file in &self.files {
            if !seen.insert(file.path.clone()) {
                return Err(ArtifactValidationError::DuplicatePath(file.path.clone()));
            }
        }

        Ok(())
    }
}

/// Validation failures for [`ArtifactSet`].
#[derive(Debug, Error)]
pub enum ArtifactValidationError {
    #[error("duplicate artifact path `{0}`")]
    DuplicatePath(String),
}
