use crate::scenario::{ExistingCluster, ExternalNodeSource, ScenarioSources};

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
    pub fn try_from_sources(
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
    use crate::scenario::{ExistingCluster, ScenarioSources};

    #[test]
    fn attached_sources_are_planned() {
        let sources =
            ScenarioSources::attached(ExistingCluster::compose(vec!["node-0".to_string()]));
        let plan = SourceOrchestrationPlan::try_from_sources(&sources)
            .expect("attached sources should build a source orchestration plan");

        assert!(matches!(
            plan.mode(),
            SourceOrchestrationMode::Attached { .. }
        ));
    }
}

fn mode_from_sources(sources: &ScenarioSources) -> SourceOrchestrationMode {
    match sources {
        ScenarioSources::Managed { external } => SourceOrchestrationMode::Managed {
            managed: ManagedSource::DeployerManaged,
            external: external.clone(),
        },
        ScenarioSources::Attached { attach, external } => SourceOrchestrationMode::Attached {
            attach: attach.clone(),
            external: external.clone(),
        },
        ScenarioSources::ExternalOnly { external } => SourceOrchestrationMode::ExternalOnly {
            external: external.clone(),
        },
    }
}
