use async_trait::async_trait;

use super::{Application, DynError, Expectation, RunContext, runtime::context::RunMetrics};

#[async_trait]
/// Describes an action sequence executed during a scenario run.
pub trait Workload<E: Application>: Send + Sync {
    fn name(&self) -> &str;

    fn expectations(&self) -> Vec<Box<dyn Expectation<E>>> {
        Vec::new()
    }

    fn init(
        &mut self,
        _descriptors: &E::Deployment,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        Ok(())
    }

    async fn start(&self, ctx: &RunContext<E>) -> Result<(), DynError>;
}
