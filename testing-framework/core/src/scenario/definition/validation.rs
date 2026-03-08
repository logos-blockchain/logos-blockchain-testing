use std::time::Duration;

use tracing::debug;

use super::model::ScenarioBuildError;
use crate::scenario::{
    Application, ClusterControlProfile, ClusterMode, DynError, RequiresNodeControl,
    expectation::Expectation,
    runtime::{
        context::RunMetrics,
        orchestration::{SourceOrchestrationPlan, SourceOrchestrationPlanError},
    },
    sources::ScenarioSources,
    workload::Workload,
};

const MIN_EXPECTATION_FALLBACK_SECS: u64 = 10;
const MIN_RUN_DURATION_SECS: u64 = 10;

pub(super) fn build_source_orchestration_plan(
    sources: &ScenarioSources,
) -> Result<SourceOrchestrationPlan, ScenarioBuildError> {
    SourceOrchestrationPlan::try_from_sources(sources).map_err(source_plan_error_to_build_error)
}

pub(super) fn validate_source_contract<Caps>(
    sources: &ScenarioSources,
) -> Result<(), ScenarioBuildError>
where
    Caps: RequiresNodeControl,
{
    validate_external_only_sources(sources)?;
    validate_node_control_profile::<Caps>(sources)?;
    Ok(())
}

fn source_plan_error_to_build_error(error: SourceOrchestrationPlanError) -> ScenarioBuildError {
    match error {
        SourceOrchestrationPlanError::SourceModeNotWiredYet { mode } => {
            ScenarioBuildError::SourceModeNotWiredYet { mode }
        }
    }
}

pub(super) fn initialize_components<E: Application>(
    descriptors: &E::Deployment,
    run_metrics: &RunMetrics,
    workloads: &mut [Box<dyn Workload<E>>],
    expectations: &mut [Box<dyn Expectation<E>>],
) -> Result<(), ScenarioBuildError> {
    initialize_workloads(descriptors, run_metrics, workloads)?;
    initialize_expectations(descriptors, run_metrics, expectations)?;
    Ok(())
}

fn initialize_workloads<E: Application>(
    descriptors: &E::Deployment,
    run_metrics: &RunMetrics,
    workloads: &mut [Box<dyn Workload<E>>],
) -> Result<(), ScenarioBuildError> {
    for workload in workloads {
        debug!(workload = workload.name(), "initializing workload");
        let name = workload.name().to_owned();

        workload
            .init(descriptors, run_metrics)
            .map_err(|source| workload_init_error(name, source))?;
    }

    Ok(())
}

fn initialize_expectations<E: Application>(
    descriptors: &E::Deployment,
    run_metrics: &RunMetrics,
    expectations: &mut [Box<dyn Expectation<E>>],
) -> Result<(), ScenarioBuildError> {
    for expectation in expectations {
        debug!(expectation = expectation.name(), "initializing expectation");
        let name = expectation.name().to_owned();

        expectation
            .init(descriptors, run_metrics)
            .map_err(|source| expectation_init_error(name, source))?;
    }

    Ok(())
}

fn workload_init_error(name: String, source: DynError) -> ScenarioBuildError {
    ScenarioBuildError::WorkloadInit { name, source }
}

fn expectation_init_error(name: String, source: DynError) -> ScenarioBuildError {
    ScenarioBuildError::ExpectationInit { name, source }
}

pub(super) fn enforce_min_duration(requested: Duration) -> Duration {
    requested.max(min_run_duration())
}

fn default_expectation_cooldown() -> Duration {
    Duration::from_secs(MIN_EXPECTATION_FALLBACK_SECS)
}

pub(super) fn expectation_cooldown_for(override_value: Option<Duration>) -> Duration {
    override_value.unwrap_or_else(default_expectation_cooldown)
}

fn min_run_duration() -> Duration {
    Duration::from_secs(MIN_RUN_DURATION_SECS)
}

fn validate_external_only_sources(sources: &ScenarioSources) -> Result<(), ScenarioBuildError> {
    if matches!(sources.cluster_mode(), ClusterMode::ExternalOnly)
        && sources.external_nodes().is_empty()
    {
        return Err(ScenarioBuildError::SourceConfiguration {
            message: "external-only scenarios require at least one external node".to_owned(),
        });
    }

    Ok(())
}

fn validate_node_control_profile<Caps>(sources: &ScenarioSources) -> Result<(), ScenarioBuildError>
where
    Caps: RequiresNodeControl,
{
    let profile = sources.control_profile();

    if Caps::REQUIRED && matches!(profile, ClusterControlProfile::ExternalUncontrolled) {
        return Err(ScenarioBuildError::SourceConfiguration {
            message: format!(
                "node control is not available for cluster mode '{}' with control profile '{}'",
                sources.cluster_mode().as_str(),
                profile.as_str(),
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ScenarioBuildError, validate_external_only_sources, validate_node_control_profile,
    };
    use crate::scenario::{
        ExistingCluster, ExternalNodeSource, NodeControlCapability, sources::ScenarioSources,
    };

    #[test]
    fn external_only_requires_external_nodes() {
        let error =
            validate_external_only_sources(&ScenarioSources::default().into_external_only())
                .expect_err("external-only without nodes should fail");

        assert!(matches!(
            error,
            ScenarioBuildError::SourceConfiguration { .. }
        ));
        assert_eq!(
            error.to_string(),
            "invalid scenario source configuration: external-only scenarios require at least one external node"
        );
    }

    #[test]
    fn external_only_rejects_node_control_requirement() {
        let sources = ScenarioSources::default()
            .with_external_node(ExternalNodeSource::new(
                "node-0".to_owned(),
                "http://127.0.0.1:1".to_owned(),
            ))
            .into_external_only();
        let error = validate_node_control_profile::<NodeControlCapability>(&sources)
            .expect_err("external-only should reject node control");

        assert!(matches!(
            error,
            ScenarioBuildError::SourceConfiguration { .. }
        ));
        assert_eq!(
            error.to_string(),
            "invalid scenario source configuration: node control is not available for cluster mode 'external-only' with control profile 'external-uncontrolled'"
        );
    }

    #[test]
    fn existing_cluster_accepts_node_control_requirement() {
        let sources = ScenarioSources::default()
            .with_attach(ExistingCluster::for_compose_project("project".to_owned()));

        validate_node_control_profile::<NodeControlCapability>(&sources)
            .expect("existing cluster should be considered controllable");
    }
}
