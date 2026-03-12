use std::{collections::HashMap, sync::Mutex};

use cfgsync_core::{
    CfgsyncErrorResponse, ConfigResolveResponse, NodeArtifactsPayload, NodeConfigSource,
    NodeRegistration, RegisterNodeResponse,
};

use crate::{
    DynCfgsyncError, MaterializationResult, MaterializedArtifacts, RegistrationSnapshot,
    RegistrationSnapshotMaterializer,
};

impl RegistrationSnapshotMaterializer for MaterializedArtifacts {
    fn materialize_snapshot(
        &self,
        _registrations: &RegistrationSnapshot,
    ) -> Result<MaterializationResult, DynCfgsyncError> {
        Ok(MaterializationResult::ready(self.clone()))
    }
}

/// Registration-aware source backed by a snapshot materializer.
pub struct RegistrationConfigSource<M> {
    materializer: M,
    registrations: Mutex<HashMap<String, NodeRegistration>>,
}

impl<M> RegistrationConfigSource<M> {
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

impl<M> NodeConfigSource for RegistrationConfigSource<M>
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
        let materialized = match self.materializer.materialize_snapshot(&registrations) {
            Ok(MaterializationResult::Ready(materialized)) => materialized,
            Ok(MaterializationResult::NotReady) => {
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

        match materialized.resolve(&registration.identifier) {
            Some(config) => {
                ConfigResolveResponse::Config(NodeArtifactsPayload::from_files(config.files))
            }
            None => ConfigResolveResponse::Error(CfgsyncErrorResponse::missing_config(
                &registration.identifier,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
    use cfgsync_core::{
        CfgsyncErrorCode, ConfigResolveResponse, NodeConfigSource, NodeRegistration,
    };

    use super::RegistrationConfigSource;
    use crate::{
        CachedSnapshotMaterializer, DynCfgsyncError, MaterializationResult, MaterializedArtifacts,
        RegistrationSnapshot, RegistrationSnapshotMaterializer,
    };

    #[test]
    fn registration_source_resolves_identifier() {
        let artifacts = MaterializedArtifacts::from_nodes([(
            "node-1".to_owned(),
            ArtifactSet::new(vec![ArtifactFile::new("/config.yaml", "a: 1")]),
        )]);
        let source = RegistrationConfigSource::new(artifacts);

        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));
        let _ = source.register(registration.clone());

        match source.resolve(&registration) {
            ConfigResolveResponse::Config(payload) => assert_eq!(payload.files.len(), 1),
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    #[test]
    fn registration_source_reports_not_ready_before_registration() {
        let artifacts = MaterializedArtifacts::from_nodes([(
            "node-1".to_owned(),
            ArtifactSet::new(vec![ArtifactFile::new("/config.yaml", "a: 1")]),
        )]);
        let source = RegistrationConfigSource::new(artifacts);

        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));

        match source.resolve(&registration) {
            ConfigResolveResponse::Config(_) => panic!("expected not-ready"),
            ConfigResolveResponse::Error(error) => {
                assert!(matches!(error.code, CfgsyncErrorCode::NotReady));
            }
        }
    }

    struct ThresholdSnapshotMaterializer;

    impl RegistrationSnapshotMaterializer for ThresholdSnapshotMaterializer {
        fn materialize_snapshot(
            &self,
            registrations: &RegistrationSnapshot,
        ) -> Result<MaterializationResult, DynCfgsyncError> {
            if registrations.len() < 2 {
                return Ok(MaterializationResult::NotReady);
            }

            let nodes = registrations.iter().map(|registration| {
                (
                    registration.identifier.clone(),
                    ArtifactSet::new(vec![ArtifactFile::new(
                        "/config.yaml",
                        format!("id: {}", registration.identifier),
                    )]),
                )
            });

            Ok(MaterializationResult::ready(
                MaterializedArtifacts::from_nodes(nodes).with_shared(ArtifactSet::new(vec![
                    ArtifactFile::new("/shared.yaml", "cluster: ready"),
                ])),
            ))
        }
    }

    #[test]
    fn registration_source_materializes_from_registration_snapshot() {
        let source = RegistrationConfigSource::new(ThresholdSnapshotMaterializer);
        let node_1 = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));
        let node_2 = NodeRegistration::new("node-2", "127.0.0.2".parse().expect("parse ip"));

        let _ = source.register(node_1.clone());
        match source.resolve(&node_1) {
            ConfigResolveResponse::Config(_) => panic!("expected not-ready before threshold"),
            ConfigResolveResponse::Error(error) => {
                assert!(matches!(error.code, CfgsyncErrorCode::NotReady));
            }
        }

        let _ = source.register(node_2.clone());

        match source.resolve(&node_1) {
            ConfigResolveResponse::Config(payload) => {
                assert_eq!(payload.files.len(), 2);
                assert_eq!(payload.files[0].path, "/config.yaml");
                assert_eq!(payload.files[1].path, "/shared.yaml");
            }
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    struct CountingSnapshotMaterializer {
        calls: AtomicUsize,
    }

    impl RegistrationSnapshotMaterializer for CountingSnapshotMaterializer {
        fn materialize_snapshot(
            &self,
            registrations: &RegistrationSnapshot,
        ) -> Result<MaterializationResult, DynCfgsyncError> {
            self.calls.fetch_add(1, Ordering::SeqCst);

            Ok(MaterializationResult::ready(
                MaterializedArtifacts::from_nodes(registrations.iter().map(|registration| {
                    (
                        registration.identifier.clone(),
                        ArtifactSet::new(vec![ArtifactFile::new(
                            "/config.yaml",
                            format!("id: {}", registration.identifier),
                        )]),
                    )
                })),
            ))
        }
    }

    #[test]
    fn cached_snapshot_materializer_reuses_previous_result() {
        let source = RegistrationConfigSource::new(CachedSnapshotMaterializer::new(
            CountingSnapshotMaterializer {
                calls: AtomicUsize::new(0),
            },
        ));
        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));

        let _ = source.register(registration.clone());
        let _ = source.resolve(&registration);
        let _ = source.resolve(&registration);
    }
}
