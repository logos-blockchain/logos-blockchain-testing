use std::{collections::HashMap, sync::Mutex};

use cfgsync_core::{
    CfgsyncErrorResponse, ConfigResolveResponse, NodeArtifactsPayload, NodeConfigSource,
    NodeRegistration, RegisterNodeResponse,
};

use crate::{
    ArtifactSet, DynCfgsyncError, NodeArtifactsCatalog, NodeArtifactsMaterializer,
    RegistrationSnapshot, RegistrationSnapshotMaterializer,
};

impl NodeArtifactsMaterializer for NodeArtifactsCatalog {
    fn materialize(
        &self,
        registration: &NodeRegistration,
        _registrations: &RegistrationSnapshot,
    ) -> Result<Option<ArtifactSet>, DynCfgsyncError> {
        Ok(self
            .resolve(&registration.identifier)
            .map(build_artifact_set_from_catalog_entry))
    }
}

impl RegistrationSnapshotMaterializer for NodeArtifactsCatalog {
    fn materialize_snapshot(
        &self,
        _registrations: &RegistrationSnapshot,
    ) -> Result<Option<NodeArtifactsCatalog>, DynCfgsyncError> {
        Ok(Some(self.clone()))
    }
}

/// Registration-aware source backed by an adapter materializer.
pub struct MaterializingConfigSource<M> {
    materializer: M,
    registrations: Mutex<HashMap<String, NodeRegistration>>,
}

impl<M> MaterializingConfigSource<M> {
    #[must_use]
    pub fn new(materializer: M) -> Self {
        Self {
            materializer,
            registrations: Mutex::new(HashMap::new()),
        }
    }

    fn registration_for(&self, identifier: &str) -> Option<NodeRegistration> {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        registrations.get(identifier).cloned()
    }

    fn registration_snapshot(&self) -> RegistrationSnapshot {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        RegistrationSnapshot::new(registrations.values().cloned().collect())
    }
}

impl<M> NodeConfigSource for MaterializingConfigSource<M>
where
    M: NodeArtifactsMaterializer,
{
    fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse {
        let mut registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");
        registrations.insert(registration.identifier.clone(), registration);

        RegisterNodeResponse::Registered
    }

    fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse {
        let registration = match self.registration_for(&registration.identifier) {
            Some(registration) => registration,
            None => {
                return ConfigResolveResponse::Error(CfgsyncErrorResponse::not_ready(
                    &registration.identifier,
                ));
            }
        };
        let registrations = self.registration_snapshot();

        match self.materializer.materialize(&registration, &registrations) {
            Ok(Some(artifacts)) => ConfigResolveResponse::Config(NodeArtifactsPayload::from_files(
                artifacts.files().to_vec(),
            )),
            Ok(None) => ConfigResolveResponse::Error(CfgsyncErrorResponse::not_ready(
                &registration.identifier,
            )),
            Err(error) => ConfigResolveResponse::Error(CfgsyncErrorResponse::internal(format!(
                "failed to materialize config for host {}: {error}",
                registration.identifier
            ))),
        }
    }
}

/// Registration-aware source backed by a snapshot materializer.
pub struct SnapshotConfigSource<M> {
    materializer: M,
    registrations: Mutex<HashMap<String, NodeRegistration>>,
}

impl<M> SnapshotConfigSource<M> {
    #[must_use]
    pub fn new(materializer: M) -> Self {
        Self {
            materializer,
            registrations: Mutex::new(HashMap::new()),
        }
    }

    fn registration_for(&self, identifier: &str) -> Option<NodeRegistration> {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        registrations.get(identifier).cloned()
    }

    fn registration_snapshot(&self) -> RegistrationSnapshot {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        RegistrationSnapshot::new(registrations.values().cloned().collect())
    }
}

impl<M> NodeConfigSource for SnapshotConfigSource<M>
where
    M: RegistrationSnapshotMaterializer,
{
    fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse {
        let mut registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");
        registrations.insert(registration.identifier.clone(), registration);

        RegisterNodeResponse::Registered
    }

    fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse {
        let registration = match self.registration_for(&registration.identifier) {
            Some(registration) => registration,
            None => {
                return ConfigResolveResponse::Error(CfgsyncErrorResponse::not_ready(
                    &registration.identifier,
                ));
            }
        };

        let registrations = self.registration_snapshot();
        let catalog = match self.materializer.materialize_snapshot(&registrations) {
            Ok(Some(catalog)) => catalog,
            Ok(None) => {
                return ConfigResolveResponse::Error(CfgsyncErrorResponse::not_ready(
                    &registration.identifier,
                ));
            }
            Err(error) => {
                return ConfigResolveResponse::Error(CfgsyncErrorResponse::internal(format!(
                    "failed to materialize config snapshot: {error}"
                )));
            }
        };

        match catalog.resolve(&registration.identifier) {
            Some(config) => ConfigResolveResponse::Config(NodeArtifactsPayload::from_files(
                config.files.clone(),
            )),
            None => ConfigResolveResponse::Error(CfgsyncErrorResponse::missing_config(
                &registration.identifier,
            )),
        }
    }
}

fn build_artifact_set_from_catalog_entry(config: &crate::NodeArtifacts) -> ArtifactSet {
    ArtifactSet::new(config.files.clone())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cfgsync_artifacts::ArtifactFile;
    use cfgsync_core::{
        CfgsyncErrorCode, ConfigResolveResponse, NodeConfigSource, NodeRegistration,
    };

    use super::{MaterializingConfigSource, SnapshotConfigSource};
    use crate::{
        CachedSnapshotMaterializer, DynCfgsyncError, NodeArtifacts, NodeArtifactsCatalog,
        NodeArtifactsMaterializer, RegistrationSnapshot, RegistrationSnapshotMaterializer,
    };

    #[test]
    fn catalog_resolves_identifier() {
        let catalog = NodeArtifactsCatalog::new(vec![NodeArtifacts {
            identifier: "node-1".to_owned(),
            files: vec![ArtifactFile::new("/config.yaml", "key: value")],
        }]);

        let node = catalog.resolve("node-1").expect("resolve node config");

        assert_eq!(node.files[0].content, "key: value");
    }

    #[test]
    fn materializing_source_resolves_registered_node() {
        let catalog = NodeArtifactsCatalog::new(vec![NodeArtifacts {
            identifier: "node-1".to_owned(),
            files: vec![ArtifactFile::new("/config.yaml", "key: value")],
        }]);
        let source = MaterializingConfigSource::new(catalog);
        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));

        let _ = source.register(registration.clone());

        match source.resolve(&registration) {
            ConfigResolveResponse::Config(payload) => {
                assert_eq!(payload.files()[0].path, "/config.yaml")
            }
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    #[test]
    fn materializing_source_reports_not_ready_before_registration() {
        let catalog = NodeArtifactsCatalog::new(vec![NodeArtifacts {
            identifier: "node-1".to_owned(),
            files: vec![ArtifactFile::new("/config.yaml", "key: value")],
        }]);
        let source = MaterializingConfigSource::new(catalog);
        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));

        match source.resolve(&registration) {
            ConfigResolveResponse::Config(_) => panic!("expected not-ready error"),
            ConfigResolveResponse::Error(error) => {
                assert!(matches!(error.code, CfgsyncErrorCode::NotReady))
            }
        }
    }

    struct ThresholdMaterializer {
        calls: AtomicUsize,
    }

    impl NodeArtifactsMaterializer for ThresholdMaterializer {
        fn materialize(
            &self,
            registration: &NodeRegistration,
            registrations: &RegistrationSnapshot,
        ) -> Result<Option<crate::ArtifactSet>, DynCfgsyncError> {
            self.calls.fetch_add(1, Ordering::SeqCst);

            if registrations.len() < 2 {
                return Ok(None);
            }

            let peer_count = registrations.iter().count();
            let files = vec![
                ArtifactFile::new("/config.yaml", format!("id: {}", registration.identifier)),
                ArtifactFile::new("/peers.txt", peer_count.to_string()),
            ];

            Ok(Some(crate::ArtifactSet::new(files)))
        }
    }

    #[test]
    fn materializing_source_passes_registration_snapshot() {
        let source = MaterializingConfigSource::new(ThresholdMaterializer {
            calls: AtomicUsize::new(0),
        });
        let node_a = NodeRegistration::new("node-a", "127.0.0.1".parse().expect("parse ip"));
        let node_b = NodeRegistration::new("node-b", "127.0.0.2".parse().expect("parse ip"));

        let _ = source.register(node_a.clone());

        match source.resolve(&node_a) {
            ConfigResolveResponse::Config(_) => panic!("expected not-ready error"),
            ConfigResolveResponse::Error(error) => {
                assert!(matches!(error.code, CfgsyncErrorCode::NotReady))
            }
        }

        let _ = source.register(node_b);

        match source.resolve(&node_a) {
            ConfigResolveResponse::Config(payload) => {
                assert_eq!(payload.files()[0].content, "id: node-a");
                assert_eq!(payload.files()[1].content, "2");
            }
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }

        assert_eq!(source.materializer.calls.load(Ordering::SeqCst), 2);
    }

    struct ThresholdSnapshotMaterializer;

    impl RegistrationSnapshotMaterializer for ThresholdSnapshotMaterializer {
        fn materialize_snapshot(
            &self,
            registrations: &RegistrationSnapshot,
        ) -> Result<Option<NodeArtifactsCatalog>, DynCfgsyncError> {
            if registrations.len() < 2 {
                return Ok(None);
            }

            Ok(Some(NodeArtifactsCatalog::new(
                registrations
                    .iter()
                    .map(|registration| NodeArtifacts {
                        identifier: registration.identifier.clone(),
                        files: vec![ArtifactFile::new(
                            "/config.yaml",
                            format!("peer_count: {}", registrations.len()),
                        )],
                    })
                    .collect(),
            )))
        }
    }

    #[test]
    fn snapshot_source_materializes_from_registration_snapshot() {
        let source = SnapshotConfigSource::new(ThresholdSnapshotMaterializer);
        let node_a = NodeRegistration::new("node-a", "127.0.0.1".parse().expect("parse ip"));
        let node_b = NodeRegistration::new("node-b", "127.0.0.2".parse().expect("parse ip"));

        let _ = source.register(node_a.clone());

        match source.resolve(&node_a) {
            ConfigResolveResponse::Config(_) => panic!("expected not-ready error"),
            ConfigResolveResponse::Error(error) => {
                assert!(matches!(error.code, CfgsyncErrorCode::NotReady))
            }
        }

        let _ = source.register(node_b);

        match source.resolve(&node_a) {
            ConfigResolveResponse::Config(payload) => {
                assert_eq!(payload.files()[0].content, "peer_count: 2");
            }
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    struct CountingSnapshotMaterializer {
        calls: std::sync::Arc<AtomicUsize>,
    }

    impl RegistrationSnapshotMaterializer for CountingSnapshotMaterializer {
        fn materialize_snapshot(
            &self,
            registrations: &RegistrationSnapshot,
        ) -> Result<Option<NodeArtifactsCatalog>, DynCfgsyncError> {
            self.calls.fetch_add(1, Ordering::SeqCst);

            Ok(Some(NodeArtifactsCatalog::new(
                registrations
                    .iter()
                    .map(|registration| NodeArtifacts {
                        identifier: registration.identifier.clone(),
                        files: vec![ArtifactFile::new("/config.yaml", "cached: true")],
                    })
                    .collect(),
            )))
        }
    }

    #[test]
    fn cached_snapshot_materializer_reuses_previous_result() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let source = SnapshotConfigSource::new(CachedSnapshotMaterializer::new(
            CountingSnapshotMaterializer {
                calls: std::sync::Arc::clone(&calls),
            },
        ));
        let node_a = NodeRegistration::new("node-a", "127.0.0.1".parse().expect("parse ip"));
        let node_b = NodeRegistration::new("node-b", "127.0.0.2".parse().expect("parse ip"));

        let _ = source.register(node_a.clone());
        let _ = source.register(node_b.clone());

        let _ = source.resolve(&node_a);
        let _ = source.resolve(&node_b);

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
