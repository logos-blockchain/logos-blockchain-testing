# Extending the Framework

This guide shows how to extend the framework with custom workloads, expectations, runners, and topology helpers. Each section includes the trait outline and a minimal code example.

## Adding a Workload

**Steps:**
1. Implement `testing_framework_core::scenario::Workload`
2. Provide a name and any bundled expectations
3. Use `init` to derive inputs from topology/metrics; fail fast if prerequisites missing
4. Use `start` to drive async traffic using `RunContext` clients
5. Expose from `testing-framework/workflows` and optionally add a DSL helper

**Trait outline:**

```rust
use async_trait::async_trait;
use testing_framework_core::scenario::{
    DynError, Expectation, RunContext, RunMetrics, Workload,
};
use testing_framework_core::topology::generation::GeneratedTopology;

pub struct MyWorkload {
    // Configuration fields
    target_rate: u64,
}

impl MyWorkload {
    pub fn new(target_rate: u64) -> Self {
        Self { target_rate }
    }
}

#[async_trait]
impl Workload for MyWorkload {
    fn name(&self) -> &str {
        "my_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        // Return bundled expectations that should run with this workload
        vec![Box::new(MyExpectation::new(self.target_rate))]
    }

    fn init(
        &mut self,
        topology: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        // Validate prerequisites (e.g., enough nodes, wallet data present)
        if topology.validators().is_empty() {
            return Err("no validators available".into());
        }
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        // Drive async activity: submit transactions, query nodes, etc.
        let clients = ctx.node_clients().validator_clients();
        
        for client in clients {
            let info = client.consensus_info().await?;
            tracing::info!(?info, "workload queried node");
        }
        
        Ok(())
    }
}
```

**Key points:**
- `name()` identifies the workload in logs
- `expectations()` bundles default checks (can be empty)
- `init()` validates topology before run starts
- `start()` executes concurrently with other workloads; it should complete before run duration expires

See [Example: New Workload & Expectation](custom-workload-example.md) for a complete, runnable example.

## Adding an Expectation

**Steps:**
1. Implement `testing_framework_core::scenario::Expectation`
2. Use `start_capture` to snapshot baseline metrics (optional)
3. Use `evaluate` to assert outcomes after workloads finish
4. Return descriptive errors; the runner aggregates them
5. Export from `testing-framework/workflows` if reusable

**Trait outline:**

```rust
use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};

pub struct MyExpectation {
    expected_value: u64,
    captured_baseline: Option<u64>,
}

impl MyExpectation {
    pub fn new(expected_value: u64) -> Self {
        Self {
            expected_value,
            captured_baseline: None,
        }
    }
}

#[async_trait]
impl Expectation for MyExpectation {
    fn name(&self) -> &str {
        "my_expectation"
    }

    async fn start_capture(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        // Optional: capture baseline state before workloads start
        let client = ctx.node_clients().validator_clients().first()
            .ok_or("no validators")?;
        
        let info = client.consensus_info().await?;
        self.captured_baseline = Some(info.current_block_id.slot);
        
        tracing::info!(baseline = self.captured_baseline, "captured baseline");
        Ok(())
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        // Assert the expected condition holds after workloads finish
        let client = ctx.node_clients().validator_clients().first()
            .ok_or("no validators")?;
        
        let info = client.consensus_info().await?;
        let final_slot = info.current_block_id.slot;
        
        let baseline = self.captured_baseline.unwrap_or(0);
        let delta = final_slot.saturating_sub(baseline);
        
        if delta < self.expected_value {
            return Err(format!(
                "expected at least {} blocks, got {}",
                self.expected_value, delta
            ).into());
        }
        
        tracing::info!(delta, "expectation passed");
        Ok(())
    }
}
```

**Key points:**
- `name()` identifies the expectation in logs
- `start_capture()` runs before workloads start (optional)
- `evaluate()` runs after workloads finish; return descriptive errors
- Expectations run sequentially; keep them fast

## Adding a Runner (Deployer)

**Steps:**
1. Implement `testing_framework_core::scenario::Deployer<Caps>` for your capability type
2. Deploy infrastructure and return a `Runner`
3. Construct `NodeClients` and spawn a `BlockFeed`
4. Build a `RunContext` and provide a `CleanupGuard` for teardown

**Trait outline:**

```rust
use async_trait::async_trait;
use testing_framework_core::scenario::{
    CleanupGuard, Deployer, DynError, Metrics, NodeClients, RunContext, Runner, Scenario,
    spawn_block_feed,
};
use testing_framework_core::topology::deployment::Topology;

pub struct MyDeployer {
    // Configuration: cluster connection details, etc.
}

impl MyDeployer {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Deployer<()> for MyDeployer {
    type Error = DynError;

    async fn deploy(&self, scenario: &Scenario<()>) -> Result<Runner, Self::Error> {
        // 1. Launch nodes using scenario.topology()
        // 2. Wait for readiness (e.g., consensus info endpoint responds)
        // 3. Build NodeClients for validators/executors
        // 4. Spawn a block feed for expectations (optional but recommended)
        // 5. Create NodeControlHandle if you support restarts (optional)
        // 6. Return a Runner wrapping RunContext + CleanupGuard

        tracing::info!("deploying scenario with MyDeployer");

        let topology: Option<Topology> = None; // Some(topology) if you spawned one
        let node_clients = NodeClients::default(); // Or NodeClients::from_topology(...)

        let (block_feed, block_feed_guard) = spawn_block_feed(&node_clients).await?;

        let telemetry = Metrics::empty(); // or Metrics::from_prometheus(...)
        let node_control = None; // or Some(Arc<dyn NodeControlHandle>)

        let context = RunContext::new(
            scenario.topology().clone(),
            topology,
            node_clients,
            scenario.duration(),
            telemetry,
            block_feed,
            node_control,
        );

        // If you also have other resources to clean up (containers/pods/etc),
        // wrap them in your own CleanupGuard implementation and call
        // CleanupGuard::cleanup(Box::new(block_feed_guard)) inside it.
        Ok(Runner::new(context, Some(Box::new(block_feed_guard))))
    }
}
```

**Key points:**
- `deploy()` must return a fully prepared `Runner`
- Block until nodes are ready before returning (avoid false negatives)
- Use a `CleanupGuard` to tear down resources on failure (and on `RunHandle` drop)
- If you want chaos workloads, also provide a `NodeControlHandle` via `RunContext`

## Adding Topology Helpers

**Steps:**
1. Extend `testing_framework_core::topology::config::TopologyBuilder` with new layouts
2. Keep defaults safe: ensure at least one participant, clamp dispersal factors
3. Consider adding configuration presets for specialized parameters

**Example:**

```rust
use testing_framework_core::topology::config::TopologyBuilder;

impl TopologyBuilder {
    /// Creates a "ring" topology where each node connects to its neighbors
    pub fn network_ring(&mut self) -> &mut Self {
        // Configure peer connections in a ring layout
        self.with_network_layout(|layout| {
            // Implement ring connection logic
            layout.ring_peers()
        });
        self
    }

    /// Preset for high-throughput DA configuration
    pub fn da_high_throughput(&mut self) -> &mut Self {
        self.with_da_params(|params| {
            params
                .dispersal_factor(8)
                .replication_factor(16)
                .chunk_size(4096)
        });
        self
    }
}
```

**Key points:**
- Maintain method chaining (return `&mut Self`)
- Validate inputs: clamp factors, enforce minimums
- Document assumptions (e.g., "requires at least 4 nodes")

## Adding a DSL Helper

To expose your custom workload through the high-level DSL, add a trait extension:

```rust
use testing_framework_core::scenario::Builder as ScenarioBuilder;

pub trait MyWorkloadDsl {
    fn my_workload_with(
        self,
        f: impl FnOnce(MyWorkloadBuilder) -> MyWorkloadBuilder,
    ) -> Self;
}

impl<Caps> MyWorkloadDsl for ScenarioBuilder<Caps> {
    fn my_workload_with(
        self,
        f: impl FnOnce(MyWorkloadBuilder) -> MyWorkloadBuilder,
    ) -> Self {
        let builder = f(MyWorkloadBuilder::default());
        self.with_workload(builder.build())
    }
}
```

Users can then call:

```rust
ScenarioBuilder::topology_with(|t| { /* ... */ })
    .my_workload_with(|w| {
        w.target_rate(10)
         .some_option(true)
    })
    .build()
```

## See Also

- [API Levels: Builder DSL vs. Direct](api-levels.md) - Understanding the two API levels
- [Custom Workload Example](custom-workload-example.md) - Complete runnable example
- [Internal Crate Reference](internal-crate-reference.md) - Where to add new code
