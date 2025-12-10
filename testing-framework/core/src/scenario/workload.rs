use async_trait::async_trait;

use super::{DynError, Expectation, RunContext, runtime::context::RunMetrics};
use crate::topology::generation::GeneratedTopology;

#[async_trait]
/// Describes an action sequence executed during a scenario run.
pub trait Workload: Send + Sync {
    fn name(&self) -> &str;

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        Vec::new()
    }

    fn init(
        &mut self,
        _descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError>;
}
