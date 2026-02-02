use async_trait::async_trait;

use super::runner::Runner;
use crate::scenario::{Application, DynError, Scenario};

/// Error returned when executing workloads or expectations.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("workload failure: {0}")]
    Workload(#[source] DynError),
    #[error("expectation capture failed: {0}")]
    ExpectationCapture(#[source] DynError),
    #[error("expectations failed:\n{0}")]
    Expectations(#[source] DynError),
}

/// Deploys a scenario into a target environment and returns a `Runner`.
#[async_trait]
pub trait Deployer<E: Application, Caps = ()>: Send + Sync {
    type Error;

    async fn deploy(&self, scenario: &Scenario<E, Caps>) -> Result<Runner<E>, Self::Error>;
}
