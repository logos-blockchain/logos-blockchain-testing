# Node Control & RunContext

The deployer supplies a `RunContext` that workloads and expectations share. It
provides:

- Topology descriptors (`GeneratedTopology`)
- Client handles (`NodeClients` / `ClusterClient`) for HTTP/RPC calls
- Metrics (`RunMetrics`, `Metrics`) and block feed
- Optional `NodeControlHandle` for managing nodes

## Current Chaos Capabilities and Limitations

The framework currently supports **process-level chaos** (node restarts) for
resilience testing:

**Supported:**
- Restart validators (`restart_validator`)
- Restart executors (`restart_executor`)
- Random restart workload via `.chaos().restart()`

**Not Yet Supported:**
- Network partitions (blocking peers, packet loss)
- Resource constraints (CPU throttling, memory limits)
- Byzantine behavior injection (invalid blocks, bad signatures)
- Selective peer blocking/unblocking

For network partition testing, see [Extension Ideas](examples-advanced.md#extension-ideas)
which describes the proposed `block_peer`/`unblock_peer` API (not yet implemented).

## Accessing node control in workloads/expectations

Check for control support and use it conditionally:

```rust
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
            // Restart the first validator (index 0) if supported.
            control.restart_validator(0).await?;
        }
        Ok(())
    }
}
```

When chaos workloads need control, require `enable_node_control()` in the
scenario builder and deploy with a runner that supports it.

## Current API surface

The `NodeControlHandle` trait currently provides:

```rust
use async_trait::async_trait;
use testing_framework_core::scenario::DynError;

#[async_trait]
pub trait NodeControlHandle: Send + Sync {
    async fn restart_validator(&self, index: usize) -> Result<(), DynError>;
    async fn restart_executor(&self, index: usize) -> Result<(), DynError>;
}
```

Future extensions may include peer blocking/unblocking or other control
operations. For now, focus on restart-based chaos patterns as shown in the
chaos workload examples.

## Considerations

- Always guard control usage: not all runners expose `NodeControlHandle`.
- Treat control as best-effort: failures should surface as test failures, but
  workloads should degrade gracefully when control is absent.
- Combine control actions with expectations (e.g., restart then assert height
  convergence) to keep scenarios meaningful.
