use crate::scenario::{ClusterMode, ExistingCluster, ExternalNodeSource, sources::ScenarioSources};

/// Explicit descriptor for managed node sourcing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedSource {
    /// Nodes are provisioned by the configured deployer.
    DeployerManaged,
}

/// Internal source-orchestration mode derived from scenario source
/// configuration.
///
/// This is scaffolding-only and is intentionally not executed by deployers
/// yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceOrchestrationMode {
    Managed {
        managed: ManagedSource,
        external: Vec<ExternalNodeSource>,
    },
    Attached {
        attach: ExistingCluster,
        external: Vec<ExternalNodeSource>,
    },
    ExternalOnly {
        external: Vec<ExternalNodeSource>,
    },
}

/// Internal source-orchestration plan used to prepare future deployer wiring.
///
/// This captures only mapping-time source intent and readiness policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceOrchestrationPlan {
    mode: SourceOrchestrationMode,
}

/// Validation failure while building orchestration plan from sources.
#[derive(Debug, thiserror::Error)]
pub enum SourceOrchestrationPlanError {
    #[error("source mode '{mode}' is not wired into deployers yet")]
    SourceModeNotWiredYet { mode: &'static str },
}

impl SourceOrchestrationPlan {
    pub(crate) fn try_from_sources(
        sources: &ScenarioSources,
    ) -> Result<Self, SourceOrchestrationPlanError> {
        let mode = mode_from_sources(sources);

        Ok(Self { mode })
    }

    #[must_use]
    pub(crate) fn mode(&self) -> &SourceOrchestrationMode {
        &self.mode
    }

    #[must_use]
    pub fn external_sources(&self) -> &[ExternalNodeSource] {
        match &self.mode {
            SourceOrchestrationMode::Managed { external, .. }
            | SourceOrchestrationMode::Attached { external, .. }
            | SourceOrchestrationMode::ExternalOnly { external } => external,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SourceOrchestrationMode, SourceOrchestrationPlan};
    use crate::scenario::{ExistingCluster, ExternalNodeSource, sources::ScenarioSources};

    #[test]
    fn managed_sources_are_planned() {
        let plan = SourceOrchestrationPlan::try_from_sources(&ScenarioSources::default())
            .expect("managed sources should build a source orchestration plan");

        assert!(matches!(
            plan.mode(),
            SourceOrchestrationMode::Managed { .. }
        ));
        assert!(plan.external_sources().is_empty());
    }

    #[test]
    fn attached_sources_are_planned() {
        let sources =
            ScenarioSources::default().with_attach(ExistingCluster::for_compose_services(
                "test-project".to_string(),
                vec!["node-0".to_string()],
            ));
        let plan = SourceOrchestrationPlan::try_from_sources(&sources)
            .expect("attached sources should build a source orchestration plan");

        assert!(matches!(
            plan.mode(),
            SourceOrchestrationMode::Attached { .. }
        ));
    }

    #[test]
    fn attached_sources_keep_external_nodes() {
        let sources = ScenarioSources::default()
            .with_attach(ExistingCluster::for_compose_project(
                "test-project".to_string(),
            ))
            .with_external_node(ExternalNodeSource::new(
                "external-0".to_owned(),
                "http://127.0.0.1:1".to_owned(),
            ));
        let plan = SourceOrchestrationPlan::try_from_sources(&sources)
            .expect("attached sources with external nodes should build");

        assert!(matches!(
            plan.mode(),
            SourceOrchestrationMode::Attached { .. }
        ));
        assert_eq!(plan.external_sources().len(), 1);
        assert_eq!(plan.external_sources()[0].label(), "external-0");
    }

    #[test]
    fn external_only_sources_are_planned() {
        let sources = ScenarioSources::default()
            .with_external_node(ExternalNodeSource::new(
                "external-0".to_owned(),
                "http://127.0.0.1:1".to_owned(),
            ))
            .into_external_only();
        let plan = SourceOrchestrationPlan::try_from_sources(&sources)
            .expect("external-only sources should build a source orchestration plan");

        assert!(matches!(
            plan.mode(),
            SourceOrchestrationMode::ExternalOnly { .. }
        ));
        assert_eq!(plan.external_sources().len(), 1);
        assert_eq!(plan.external_sources()[0].label(), "external-0");
    }
}

fn mode_from_sources(sources: &ScenarioSources) -> SourceOrchestrationMode {
    match sources.cluster_mode() {
        ClusterMode::Managed => match sources {
            ScenarioSources::Managed { external } => SourceOrchestrationMode::Managed {
                managed: ManagedSource::DeployerManaged,
                external: external.clone(),
            },
            ScenarioSources::Attached { .. } | ScenarioSources::ExternalOnly { .. } => {
                unreachable!("cluster mode and source storage must stay aligned")
            }
        },
        ClusterMode::ExistingCluster => match sources {
            ScenarioSources::Attached { attach, external } => SourceOrchestrationMode::Attached {
                attach: attach.clone(),
                external: external.clone(),
            },
            ScenarioSources::Managed { .. } | ScenarioSources::ExternalOnly { .. } => {
                unreachable!("cluster mode and source storage must stay aligned")
            }
        },
        ClusterMode::ExternalOnly => match sources {
            ScenarioSources::ExternalOnly { external } => SourceOrchestrationMode::ExternalOnly {
                external: external.clone(),
            },
            ScenarioSources::Managed { .. } | ScenarioSources::Attached { .. } => {
                unreachable!("cluster mode and source storage must stay aligned")
            }
        },
    }
}
