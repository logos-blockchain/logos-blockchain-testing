use std::time::Duration;

use tracing::debug;

use super::model::ScenarioBuildError;
use crate::scenario::{
    Application, DynError, expectation::Expectation, runtime::context::RunMetrics,
    workload::Workload,
};

const MIN_EXPECTATION_FALLBACK_SECS: u64 = 10;
const MIN_RUN_DURATION_SECS: u64 = 10;

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
