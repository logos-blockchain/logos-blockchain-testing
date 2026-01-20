use async_trait::async_trait;
use testing_framework_core::{
    scenario::{DynError, Expectation, RunContext, RunMetrics, Workload},
    topology::generation::GeneratedTopology,
};

pub struct ReachabilityWorkload {
    target_idx: usize,
}

impl ReachabilityWorkload {
    pub fn new(target_idx: usize) -> Self {
        Self { target_idx }
    }
}

#[async_trait]
impl Workload for ReachabilityWorkload {
    fn name(&self) -> &str {
        "reachability_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        vec![Box::new(
            crate::custom_workload_example_expectation::ReachabilityExpectation::new(
                self.target_idx,
            ),
        )]
    }

    fn init(
        &mut self,
        topology: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        if topology.nodes().get(self.target_idx).is_none() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "no node at requested index",
            )));
        }
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let clients = ctx.node_clients().node_clients();
        let client = clients.get(self.target_idx).ok_or_else(|| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "missing target client",
            )) as DynError
        })?;

        // Lightweight API call to prove reachability.
        client
            .consensus_info()
            .await
            .map(|_| ())
            .map_err(|e| e.into())
    }
}
