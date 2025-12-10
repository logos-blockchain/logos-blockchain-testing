use async_trait::async_trait;

use super::{DynError, RunContext, runtime::context::RunMetrics};
use crate::topology::generation::GeneratedTopology;

#[async_trait]
/// Defines a check evaluated during or after a scenario run.
pub trait Expectation: Send + Sync {
    fn name(&self) -> &str;

    fn init(
        &mut self,
        _descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        Ok(())
    }

    async fn start_capture(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        Ok(())
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError>;
}
