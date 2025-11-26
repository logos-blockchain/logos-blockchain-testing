# Example: New Workload & Expectation (Rust)

A minimal, end-to-end illustration of adding a custom workload and matching
expectation. This shows the shape of the traits and where to plug into the
framework; expand the logic to fit your real test.

## Workload: simple reachability probe

Key ideas:
- **name**: identifies the workload in logs.
- **expectations**: workloads can bundle defaults so callers don’t forget checks.
- **init**: derive inputs from the generated topology (e.g., pick a target node).
- **start**: drive async activity using the shared `RunContext`.

```rust
use std::sync::Arc;
use async_trait::async_trait;
use testing_framework_core::scenario::{
    DynError, Expectation, RunContext, RunMetrics, Workload,
};
use testing_framework_core::topology::GeneratedTopology;

pub struct ReachabilityWorkload {
    target_idx: usize,
    bundled: Vec<Box<dyn Expectation>>,
}

impl ReachabilityWorkload {
    pub fn new(target_idx: usize) -> Self {
        Self {
            target_idx,
            bundled: vec![Box::new(ReachabilityExpectation::new(target_idx))],
        }
    }
}

#[async_trait]
impl Workload for ReachabilityWorkload {
    fn name(&self) -> &'static str {
        "reachability_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        self.bundled.clone()
    }

    fn init(
        &mut self,
        topology: &GeneratedTopology,
        _metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        if topology.validators().get(self.target_idx).is_none() {
            return Err("no validator at requested index".into());
        }
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let client = ctx
            .clients()
            .validators()
            .get(self.target_idx)
            .ok_or("missing target client")?;

        // Pseudo-action: issue a lightweight RPC to prove reachability.
        client.health_check().await.map_err(|e| e.into())
    }
}
```

## Expectation: confirm the target stayed reachable

Key ideas:
- **start_capture**: snapshot baseline if needed (not used here).
- **evaluate**: assert the condition after workloads finish.

```rust
use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};

pub struct ReachabilityExpectation {
    target_idx: usize,
}

impl ReachabilityExpectation {
    pub fn new(target_idx: usize) -> Self {
        Self { target_idx }
    }
}

#[async_trait]
impl Expectation for ReachabilityExpectation {
    fn name(&self) -> &str {
        "target_reachable"
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        let client = ctx
            .clients()
            .validators()
            .get(self.target_idx)
            .ok_or("missing target client")?;

        client.health_check().await.map_err(|e| {
            format!("target became unreachable during run: {e}").into()
        })
    }
}
```

## How to wire it
- Build your scenario as usual and call `.with_workload(ReachabilityWorkload::new(0))`.
- The bundled expectation is attached automatically; you can add more with
  `.with_expectation(...)` if needed.
- Keep the logic minimal and fast for smoke tests; grow it into richer probes
  for deeper scenarios.
