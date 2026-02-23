use std::fmt;

use crate::scenario::{AttachSource, ExternalNodeSource, ScenarioSources, SourceReadinessPolicy};

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
pub enum SourceOrchestrationMode {
    Managed {
        managed: ManagedSource,
        external: Vec<ExternalNodeSource>,
    },
    Attached {
        attach: AttachSource,
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
    pub mode: SourceOrchestrationMode,
    pub readiness_policy: SourceReadinessPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceModeName {
    Attached,
    ExternalOnly,
}

impl fmt::Display for SourceModeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attached => f.write_str("Attached"),
            Self::ExternalOnly => f.write_str("ExternalOnly"),
        }
    }
}

/// Validation failure while building orchestration plan from sources.
#[derive(Debug, thiserror::Error)]
pub enum SourceOrchestrationPlanError {
    #[error("managed source selected but deployment produced 0 managed nodes")]
    ManagedNodesMissing,
    #[error("source mode '{mode}' is not wired into deployers yet")]
    SourceModeNotWiredYet { mode: SourceModeName },
}

impl SourceOrchestrationPlan {
    pub fn try_from_sources(
        sources: &ScenarioSources,
        managed_node_count: usize,
        readiness_policy: SourceReadinessPolicy,
    ) -> Result<Self, SourceOrchestrationPlanError> {
        ensure_managed_sources_have_nodes(sources, managed_node_count)?;
        let mode = mode_from_sources(sources);

        let plan = Self {
            mode,
            readiness_policy,
        };

        plan.ensure_currently_wired()?;
        Ok(plan)
    }

    #[must_use]
    pub fn external_sources(&self) -> &[ExternalNodeSource] {
        match &self.mode {
            SourceOrchestrationMode::Managed { external, .. }
            | SourceOrchestrationMode::Attached { external, .. }
            | SourceOrchestrationMode::ExternalOnly { external } => external,
        }
    }

    fn ensure_currently_wired(&self) -> Result<(), SourceOrchestrationPlanError> {
        match self.mode {
            SourceOrchestrationMode::Managed { .. } => Ok(()),
            SourceOrchestrationMode::Attached { .. } => not_wired(SourceModeName::Attached),
            SourceOrchestrationMode::ExternalOnly { .. } => not_wired(SourceModeName::ExternalOnly),
        }
    }
}

fn ensure_managed_sources_have_nodes(
    sources: &ScenarioSources,
    managed_node_count: usize,
) -> Result<(), SourceOrchestrationPlanError> {
    if matches!(sources, ScenarioSources::Managed { .. }) && managed_node_count == 0 {
        return Err(SourceOrchestrationPlanError::ManagedNodesMissing);
    }

    Ok(())
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

fn not_wired(mode: SourceModeName) -> Result<(), SourceOrchestrationPlanError> {
    Err(SourceOrchestrationPlanError::SourceModeNotWiredYet { mode })
}
