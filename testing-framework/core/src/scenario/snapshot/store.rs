use std::{
    fs, io,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

use crate::scenario::{DynError, NodeStateSource, SnapshotArtifact, SnapshotManifest};

const MANIFEST_FILE: &str = "snapshot-manifest.json";

#[derive(Clone, Debug)]
/// Filesystem-backed snapshot store.
///
/// The store owns the on-disk layout under a root directory. It is
/// intentionally generic over the concrete node state directories: callers
/// provide the list of subdirectories that make up durable node state.
pub struct SnapshotStore {
    root_dir: PathBuf,
}

impl SnapshotStore {
    /// Create a snapshot store rooted at `root_dir`.
    #[must_use]
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
        }
    }

    /// Root directory containing named snapshots.
    #[must_use]
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// Save selected node state subdirectories into a named snapshot.
    ///
    /// The destination layout is `<root>/<snapshot_name>/<node_name>/<subdir>`.
    /// If a manifest already exists it is updated; an invalid existing manifest
    /// fails the save instead of being overwritten.
    pub fn save_node_dirs(
        &self,
        snapshot_name: &str,
        node_name: &str,
        runtime_dir: &Path,
        state_subdirs: &[&str],
    ) -> Result<String, DynError> {
        validate_path_component(snapshot_name, "Snapshot name")?;
        validate_path_component(node_name, "Node name")?;

        let mut manifest = self.read_manifest_or_new(snapshot_name)?;
        let destination = self.snapshot_node_dir(snapshot_name, node_name);
        clear_dir(&destination)?;

        for dir_name in state_subdirs {
            copy_dir_recursive(&runtime_dir.join(dir_name), &destination.join(dir_name)).map_err(
                |source| {
                    io::Error::other(format!(
                        "failed to copy node `{node_name}` state from '{}' to '{}': {source}",
                        runtime_dir.display(),
                        destination.display()
                    ))
                },
            )?;
        }

        manifest.node_state.insert(
            node_name.to_owned(),
            SnapshotArtifact::new(
                1,
                serde_json::json!({
                    "state_subdirs": state_subdirs,
                }),
                serde_json::json!({
                    "artifact": node_name,
                }),
            ),
        );
        self.write_manifest(&manifest)?;

        Ok(snapshot_name.to_owned())
    }

    /// Resolve and validate a node-state source directory.
    ///
    /// This does not copy files. It only returns the source path after checking
    /// that all required state subdirectories exist.
    pub fn prepare_node_dir(
        &self,
        source: &NodeStateSource,
        required_subdirs: &[&str],
    ) -> Result<PathBuf, DynError> {
        let source_dir = match source {
            NodeStateSource::Snapshot { snapshot, node } => self.snapshot_node_dir(snapshot, node),
            NodeStateSource::ExternalDirectory(path) => path.clone(),
        };

        ensure_subdirs(&source_dir, required_subdirs)?;

        Ok(source_dir)
    }

    /// Restore node state subdirectories into a runtime directory.
    ///
    /// Each target subdirectory is cleared before the corresponding source
    /// subdirectory is copied.
    pub fn restore_node_dirs(
        &self,
        source: &NodeStateSource,
        runtime_dir: &Path,
        state_subdirs: &[&str],
    ) -> Result<(), DynError> {
        let source_dir = self.prepare_node_dir(source, state_subdirs)?;

        for dir_name in state_subdirs {
            let runtime_state_dir = runtime_dir.join(dir_name);
            clear_dir(&runtime_state_dir)?;
            copy_dir_recursive(&source_dir.join(dir_name), &runtime_state_dir)?;
        }

        Ok(())
    }

    /// Save or replace one provider artifact in a named snapshot manifest.
    ///
    /// If the snapshot manifest does not exist it is created. Invalid existing
    /// manifests fail the save instead of being overwritten.
    pub fn save_provider_artifact(
        &self,
        snapshot_name: &str,
        provider_id: &str,
        artifact: SnapshotArtifact,
    ) -> Result<String, DynError> {
        validate_path_component(snapshot_name, "Snapshot name")?;
        validate_path_component(provider_id, "Snapshot provider id")?;

        let mut manifest = self.read_manifest_or_new(snapshot_name)?;

        manifest.providers.insert(provider_id.to_owned(), artifact);
        self.write_manifest(&manifest)?;

        Ok(snapshot_name.to_owned())
    }

    /// Load one provider artifact from a named snapshot manifest.
    pub fn load_provider_artifact(
        &self,
        snapshot_name: &str,
        provider_id: &str,
    ) -> Result<SnapshotArtifact, DynError> {
        validate_path_component(snapshot_name, "Snapshot name")?;
        validate_path_component(provider_id, "Snapshot provider id")?;

        let manifest = self.read_manifest(snapshot_name)?;
        manifest.providers.get(provider_id).cloned().ok_or_else(|| {
            format!("snapshot '{snapshot_name}' does not contain provider '{provider_id}'").into()
        })
    }

    /// Read a snapshot manifest by snapshot name.
    pub fn read_manifest(&self, snapshot: &str) -> Result<SnapshotManifest, DynError> {
        let path = self.manifest_path(snapshot);
        let bytes = fs::read(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to read snapshot manifest at '{}': {error}",
                    path.display()
                ),
            )
        })?;
        parse_manifest(&path, &bytes)
    }

    fn read_manifest_or_new(&self, snapshot: &str) -> Result<SnapshotManifest, DynError> {
        let path = self.manifest_path(snapshot);
        match fs::read(&path) {
            Ok(bytes) => parse_manifest(&path, &bytes),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Ok(SnapshotManifest::new(snapshot))
            }
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!(
                    "failed to read snapshot manifest at '{}': {error}",
                    path.display()
                ),
            )
            .into()),
        }
    }

    /// Write a snapshot manifest to its canonical path.
    pub fn write_manifest(&self, manifest: &SnapshotManifest) -> Result<(), DynError> {
        validate_path_component(&manifest.name, "Snapshot name")?;

        let path = self.manifest_path(&manifest.name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(manifest)?)?;
        Ok(())
    }

    /// Return the canonical directory for one node inside a named snapshot.
    #[must_use]
    pub fn snapshot_node_dir(&self, snapshot: &str, node_name: &str) -> PathBuf {
        self.root_dir.join(snapshot).join(node_name)
    }

    /// Return the canonical manifest path for a named snapshot.
    #[must_use]
    pub fn manifest_path(&self, snapshot: &str) -> PathBuf {
        self.root_dir.join(snapshot).join(MANIFEST_FILE)
    }
}

fn validate_path_component(value: &str, field_name: &str) -> Result<(), DynError> {
    if value.trim().is_empty() {
        return Err(format!("{field_name} cannot be empty").into());
    }

    let path = Path::new(value);
    let mut components = path.components();

    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) if !path.is_absolute() => Ok(()),
        _ => {
            Err(format!("{field_name} must be a single safe path component, got `{value}`").into())
        }
    }
}

fn ensure_subdirs(path: &Path, required_subdirs: &[&str]) -> Result<(), DynError> {
    for dir_name in required_subdirs {
        let state_dir = path.join(dir_name);
        if !state_dir.is_dir() {
            return Err(format!(
                "snapshot state path '{}' is missing or is not a directory",
                state_dir.display()
            )
            .into());
        }
    }

    Ok(())
}

fn parse_manifest(path: &Path, bytes: &[u8]) -> Result<SnapshotManifest, DynError> {
    serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "failed to parse snapshot manifest at '{}': {error}",
            path.display()
        )
        .into()
    })
}

fn clear_dir(path: &Path) -> io::Result<()> {
    if path.exists() {
        if !path.is_dir() {
            return Err(io::Error::other(format!(
                "path exists but is not a directory: '{}'",
                path.display()
            )));
        }
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if !src.is_dir() {
        return Err(io::Error::other(format!(
            "directory '{}' is missing",
            src.display()
        )));
    }

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = dst.join(entry.file_name());
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &destination_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir should be created");
        }
        fs::write(path, contents).expect("test file should be written");
    }

    #[test]
    fn node_state_and_provider_artifacts_share_manifest() {
        let root = TempDir::new().expect("snapshot root should be created");
        let runtime = TempDir::new().expect("runtime dir should be created");
        let store = SnapshotStore::new(root.path());

        write_file(&runtime.path().join("db").join("state.txt"), "db");

        store
            .save_provider_artifact(
                "snapshot",
                "wallet",
                SnapshotArtifact::new(
                    1,
                    serde_json::json!({ "wallet_count": 1 }),
                    serde_json::json!({ "wallets": ["WALLET_1A"] }),
                ),
            )
            .expect("provider artifact should save");

        store
            .save_node_dirs("snapshot", "NODE_1", runtime.path(), &["db"])
            .expect("node state should save");

        let manifest = store
            .read_manifest("snapshot")
            .expect("manifest should be readable");

        assert!(manifest.node_state.contains_key("NODE_1"));
        assert_eq!(
            manifest
                .providers
                .get("wallet")
                .expect("wallet artifact should remain in manifest")
                .metadata,
            serde_json::json!({ "wallet_count": 1 })
        );
        assert!(
            store
                .snapshot_node_dir("snapshot", "NODE_1")
                .join("db")
                .join("state.txt")
                .is_file()
        );
    }

    #[test]
    fn external_directory_node_state_source_resolves_existing_state() {
        let root = TempDir::new().expect("snapshot root should be created");
        let external = TempDir::new().expect("external dir should be created");
        let store = SnapshotStore::new(root.path());

        fs::create_dir_all(external.path().join("db")).expect("db dir should be created");
        fs::create_dir_all(external.path().join("recovery"))
            .expect("recovery dir should be created");

        let source = NodeStateSource::external_directory(external.path());
        let resolved = store
            .prepare_node_dir(&source, &["db", "recovery"])
            .expect("external source should resolve");

        assert_eq!(resolved, external.path());
    }

    #[test]
    fn external_directory_node_state_source_requires_state_subdirs() {
        let root = TempDir::new().expect("snapshot root should be created");
        let external = TempDir::new().expect("external dir should be created");
        let store = SnapshotStore::new(root.path());

        fs::create_dir_all(external.path().join("db")).expect("db dir should be created");

        let source = NodeStateSource::external_directory(external.path());
        let error = store
            .prepare_node_dir(&source, &["db", "recovery"])
            .expect_err("missing required state dir should fail");

        assert!(error.to_string().contains("recovery"));
    }

    #[test]
    fn saving_node_state_does_not_overwrite_invalid_manifest() {
        let root = TempDir::new().expect("snapshot root should be created");
        let runtime = TempDir::new().expect("runtime dir should be created");
        let store = SnapshotStore::new(root.path());

        write_file(&runtime.path().join("db").join("state.txt"), "db");
        write_file(&store.manifest_path("snapshot"), "not json");

        let error = store
            .save_node_dirs("snapshot", "NODE_1", runtime.path(), &["db"])
            .expect_err("invalid manifest should fail the save");

        assert!(
            error
                .to_string()
                .contains("failed to parse snapshot manifest")
        );
        assert!(error.to_string().contains("expected ident"));
        assert_eq!(
            fs::read_to_string(store.manifest_path("snapshot"))
                .expect("manifest should still exist"),
            "not json"
        );
        assert!(
            !store.snapshot_node_dir("snapshot", "NODE_1").exists(),
            "node state should not be copied when manifest cannot be read"
        );
    }
}
