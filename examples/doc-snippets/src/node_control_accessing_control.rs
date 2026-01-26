use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, RunContext, Workload};

struct RestartWorkload;

#[async_trait]
impl Workload for RestartWorkload {
    fn name(&self) -> &str {
        "restart_workload"
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        if let Some(control) = ctx.node_control() {
            // Restart the first node (index 0) if supported.
            control.restart_node(0).await?;
        }
        Ok(())
    }
}
