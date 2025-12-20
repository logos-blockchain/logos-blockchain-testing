# RunContext: BlockFeed & Node Control

The deployer supplies a `RunContext` that workloads and expectations share. It
provides:

- Topology descriptors (`GeneratedTopology`)
- Client handles (`NodeClients` / `ClusterClient`) for HTTP/RPC calls
- Metrics (`RunMetrics`, `Metrics`) and block feed
- Optional `NodeControlHandle` for managing nodes

## BlockFeed: Observing Block Production

The `BlockFeed` is a broadcast stream of block observations that allows workloads and expectations to monitor blockchain progress in real-time. It polls a validator node continuously and broadcasts new blocks to all subscribers.

### What BlockFeed Provides

**Real-time block stream:**
- Subscribe to receive `BlockRecord` notifications as blocks are produced
- Each record includes the block header (`HeaderId`) and full block payload
- Backed by a background task that polls node storage every second

**Block statistics:**
- Track total transactions across all observed blocks
- Access via `block_feed.stats().total_transactions()`

**Broadcast semantics:**
- Multiple subscribers can receive the same blocks independently
- Late subscribers start receiving from current block (no history replay)
- Lagged subscribers skip missed blocks automatically

### Accessing BlockFeed

BlockFeed is available through `RunContext`:

```rust,ignore
let block_feed = ctx.block_feed();
```

### Usage in Expectations

Expectations typically use BlockFeed to verify block production and inclusion of transactions/data.

**Example: Counting blocks during a run**

```rust,ignore
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};

struct MinimumBlocksExpectation {
    min_blocks: u64,
    captured_blocks: Option<Arc<AtomicU64>>,
}

#[async_trait]
impl Expectation for MinimumBlocksExpectation {
    fn name(&self) -> &'static str {
        "minimum_blocks"
    }

    async fn start_capture(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        let block_count = Arc::new(AtomicU64::new(0));
        let block_count_task = Arc::clone(&block_count);
        
        // Subscribe to block feed
        let mut receiver = ctx.block_feed().subscribe();
        
        // Spawn a task to count blocks
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(_record) => {
                        block_count_task.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "receiver lagged, skipping blocks");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::debug!("block feed closed");
                        break;
                    }
                }
            }
        });
        
        self.captured_blocks = Some(block_count);
        Ok(())
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        let blocks = self.captured_blocks
            .as_ref()
            .expect("start_capture must be called first")
            .load(Ordering::Relaxed);
        
        if blocks < self.min_blocks {
            return Err(format!(
                "expected at least {} blocks, observed {}",
                self.min_blocks, blocks
            ).into());
        }
        
        tracing::info!(blocks, min = self.min_blocks, "minimum blocks expectation passed");
        Ok(())
    }
}
```

**Example: Inspecting block contents**

```rust,ignore
use testing_framework_core::scenario::{DynError, RunContext};

async fn start_capture(ctx: &RunContext) -> Result<(), DynError> {
    let mut receiver = ctx.block_feed().subscribe();
    
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(record) => {
                    // Access block header
                    let header_id = &record.header;
                    
                    // Access full block
                    let tx_count = record.block.transactions().len();
                    
                    tracing::debug!(
                        ?header_id,
                        tx_count,
                        "observed block"
                    );
                    
                    // Process transactions, DA blobs, etc.
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(_) => continue,
            }
        }
    });
    
    Ok(())
}
```

### Usage in Workloads

Workloads can use BlockFeed to coordinate timing or wait for specific conditions before proceeding.

**Example: Wait for N blocks before starting**

```rust,ignore
use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, RunContext, Workload};

struct DelayedWorkload {
    wait_blocks: usize,
}

#[async_trait]
impl Workload for DelayedWorkload {
    fn name(&self) -> &str {
        "delayed_workload"
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        tracing::info!(wait_blocks = self.wait_blocks, "waiting for blocks before starting");
        
        // Subscribe to block feed
        let mut receiver = ctx.block_feed().subscribe();
        let mut count = 0;
        
        // Wait for N blocks
        while count < self.wait_blocks {
            match receiver.recv().await {
                Ok(_) => count += 1,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err("block feed closed before reaching target".into());
                }
            }
        }
        
        tracing::info!("warmup complete, starting actual workload");
        
        // Now do the actual work
        // ...
        
        Ok(())
    }
}
```

**Example: Rate limiting based on block production**

```rust,ignore
use testing_framework_core::scenario::{DynError, RunContext};

async fn generate_request() -> Option<()> {
    None
}

async fn start(ctx: &RunContext) -> Result<(), DynError> {
    let clients = ctx.node_clients().validator_clients();
    let mut receiver = ctx.block_feed().subscribe();
    let mut pending_requests: Vec<()> = Vec::new();

    loop {
        tokio::select! {
            // Issue a batch on each new block.
            Ok(_record) = receiver.recv() => {
                if !pending_requests.is_empty() {
                    tracing::debug!(count = pending_requests.len(), "issuing requests on new block");
                    for _req in pending_requests.drain(..) {
                        let _info = clients[0].consensus_info().await?;
                    }
                }
            }

            // Generate work continuously.
            Some(req) = generate_request() => {
                pending_requests.push(req);
            }
        }
    }
}
```

### BlockFeed vs Direct Polling

**Use BlockFeed when:**
- You need to react to blocks as they're produced
- Multiple components need to observe the same blocks
- You want automatic retry/reconnect logic
- You're tracking statistics across many blocks

**Use direct polling when:**
- You need to query specific historical blocks
- You're checking final state after workloads complete
- You need transaction receipts or other indexed data
- You're implementing a one-time health check

Example direct polling in expectations:

```rust,ignore
use testing_framework_core::scenario::{DynError, RunContext};

async fn evaluate(ctx: &RunContext) -> Result<(), DynError> {
    let client = &ctx.node_clients().validator_clients()[0];
    
    // Poll current height once
    let info = client.consensus_info().await?;
    tracing::info!(height = info.height, "final block height");
    
    // This is simpler than BlockFeed for one-time checks
    Ok(())
}
```

### Block Statistics

Access aggregated statistics without subscribing to the feed:

```rust,ignore
use testing_framework_core::scenario::{DynError, RunContext};

async fn evaluate(ctx: &RunContext, expected_min: u64) -> Result<(), DynError> {
    let stats = ctx.block_feed().stats();
    let total_txs = stats.total_transactions();
    
    tracing::info!(total_txs, "transactions observed across all blocks");
    
    if total_txs < expected_min {
        return Err(format!(
            "expected at least {} transactions, observed {}",
            expected_min, total_txs
        ).into());
    }
    
    Ok(())
}
```

### Important Notes

**Subscription timing:**
- Subscribe in `start_capture()` for expectations
- Subscribe in `start()` for workloads
- Late subscribers miss historical blocks (no replay)

**Lagged receivers:**
- If your subscriber is too slow, it may lag behind
- Handle `RecvError::Lagged(skipped)` gracefully
- Consider increasing processing speed or reducing block rate

**Feed lifetime:**
- BlockFeed runs for the entire scenario duration
- Automatically cleaned up when the run completes
- Closed channels signal graceful shutdown

**Performance:**
- BlockFeed polls nodes every 1 second
- Broadcasts to all subscribers with minimal overhead
- Suitable for scenarios with hundreds of blocks

### Real-World Examples

The framework's built-in expectations use BlockFeed extensively:

- **`ConsensusLiveness`**: Doesn't directly subscribe but uses block feed stats to verify progress
- **`DataAvailabilityExpectation`**: Subscribes to inspect DA blobs in each block and track inscription/dispersal
- **`TransactionInclusion`**: Subscribes to find specific transactions in blocks

See [Examples](examples.md) and [Workloads & Expectations](workloads.md) for more patterns.

---

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

```rust,ignore
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

```rust,ignore
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
