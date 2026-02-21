use async_trait::async_trait;

use super::{Application, DynError, RunContext, runtime::context::RunMetrics};

#[async_trait]
/// Defines a check evaluated during or after a scenario run.
pub trait Expectation<E: Application>: Send + Sync {
    fn name(&self) -> &str;

    fn init(
        &mut self,
        _descriptors: &E::Deployment,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        Ok(())
    }

    async fn start_capture(&mut self, _ctx: &RunContext<E>) -> Result<(), DynError> {
        Ok(())
    }

    /// Optional periodic check used by fail-fast expectation mode.
    ///
    /// Default is a no-op so existing expectations stay end-of-run only.
    async fn check_during_capture(&mut self, _ctx: &RunContext<E>) -> Result<(), DynError> {
        Ok(())
    }

    async fn evaluate(&mut self, ctx: &RunContext<E>) -> Result<(), DynError>;
}
