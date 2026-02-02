use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, RunContext, Workload};

pub struct YourWorkload;

#[async_trait]
impl Workload for YourWorkload {
    fn name(&self) -> &'static str {
        "your_workload"
    }

    async fn start(&self, _ctx: &RunContext) -> Result<(), DynError> {
        // implementation
        Ok(())
    }
}
