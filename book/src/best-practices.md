# Best Practices

This page collects proven patterns for authoring, running, and maintaining test scenarios that are reliable, maintainable, and actionable.

## Scenario Design

**State your intent**
- Document the goal of each scenario (throughput, DA validation, resilience) so expectation choices are obvious
- Use descriptive variable names that explain topology purpose (e.g., `star_topology_3val_2exec` vs `topology`)
- Add comments explaining why specific rates or durations were chosen

**Keep runs meaningful**
- Choose durations that allow multiple blocks and make timing-based assertions trustworthy
- Use [FAQ: Run Duration Calculator](faq.md#how-long-should-a-scenario-run) to estimate minimum duration
- Avoid runs shorter than 30 seconds unless testing startup behavior specifically

**Separate concerns**
- Start with deterministic workloads for functional checks
- Add chaos in dedicated resilience scenarios to avoid noisy failures
- Don't mix high transaction load with aggressive chaos in the same test (hard to debug)

**Start small, scale up**
- Begin with minimal topology (1-2 validators) to validate scenario logic
- Gradually increase topology size and workload rates
- Use Host runner for fast iteration, then validate on Compose before production

## Code Organization

**Reuse patterns**
- Standardize on shared topology and workload presets so results are comparable across environments and teams
- Extract common topology builders into helper functions
- Create workspace-level constants for standard rates and durations

**Example: Topology preset**

```rust,ignore
pub fn standard_da_topology() -> GeneratedTopology {
    TopologyBuilder::new()
        .network_star()
        .validators(3)
        .generate()
}
```

**Example: Shared constants**

```rust,ignore
pub const STANDARD_TX_RATE: f64 = 10.0;
pub const STANDARD_DA_CHANNEL_RATE: f64 = 2.0;
pub const SHORT_RUN_DURATION: Duration = Duration::from_secs(60);
pub const LONG_RUN_DURATION: Duration = Duration::from_secs(300);
```

## Debugging & Observability

**Observe first, tune second**
- Rely on liveness and inclusion signals to interpret outcomes before tweaking rates or topology
- Enable detailed logging (`RUST_LOG=debug`, `NOMOS_LOG_LEVEL=debug`) only after initial failure
- Use `NOMOS_TESTS_KEEP_LOGS=1` to persist logs when debugging failures

**Use BlockFeed effectively**
- Subscribe to BlockFeed in expectations for real-time block monitoring
- Track block production rate to detect liveness issues early
- Use block statistics (`block_feed.stats().total_transactions()`) to verify inclusion

**Collect metrics**
- Set up Prometheus/Grafana via `scripts/setup/setup-observability.sh compose up` for visualizing node behavior
- Use metrics to identify bottlenecks before adding more load
- Monitor mempool size, block size, and consensus timing

## Environment & Runner Selection

**Environment fit**
- Pick runners that match the feedback loop you need:
  - **Host**: Fast iteration during development, quick CI smoke tests
  - **Compose**: Reproducible environments (recommended for CI), chaos testing
  - **K8s**: Production-like fidelity, large topologies (10+ nodes)

**Runner-specific considerations**

| Runner | When to Use | When to Avoid |
|--------|-------------|---------------|
| Host | Development iteration, fast feedback | Chaos testing, container-specific issues |
| Compose | CI pipelines, chaos tests, reproducibility | Very large topologies (>10 nodes) |
| K8s | Production-like testing, cluster behaviors | Local development, fast iteration |

**Minimal surprises**
- Seed only necessary wallets and keep configuration deltas explicit when moving between CI and developer machines
- Use `versions.env` to pin node versions consistently across environments
- Document non-default environment variables in scenario comments or README

## CI/CD Integration

**Use matrix builds**

```yaml
strategy:
  matrix:
    runner: [host, compose]
    topology: [small, medium]
```

**Cache aggressively**
- Cache Rust build artifacts (`target/`)
- Cache circuit parameters (`assets/stack/kzgrs_test_params/`)
- Cache Docker layers (use BuildKit cache)

**Collect logs on failure**

```yaml
- name: Collect logs on failure
  if: failure()
  run: |
    mkdir -p test-logs
    find /tmp -name "nomos-*.log" -exec cp {} test-logs/ \;
- uses: actions/upload-artifact@v3
  if: failure()
  with:
    name: test-logs-${{ matrix.runner }}
    path: test-logs/
```

**Time limits**
- Set job timeout to prevent hung runs: `timeout-minutes: 30`
- Use shorter durations in CI (60s) vs local testing (300s)
- Run expensive tests (k8s, large topologies) only on main branch or release tags

**See also:** [CI Integration](ci-integration.md) for complete workflow examples

## Anti-Patterns to Avoid

**DON'T: Run without POL_PROOF_DEV_MODE**
```bash
# BAD: Will hang/timeout on proof generation
cargo run -p runner-examples --bin local_runner

# GOOD: Fast mode for testing
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
```

**DON'T: Use tiny durations**
```rust,ignore
// BAD: Not enough time for blocks to propagate
.with_run_duration(Duration::from_secs(5))

// GOOD: Allow multiple consensus rounds
.with_run_duration(Duration::from_secs(60))
```

**DON'T: Ignore cleanup failures**
```rust,ignore
// BAD: Next run inherits leaked state
runner.run(&mut scenario).await?;
// forgot to call cleanup or use CleanupGuard

// GOOD: Cleanup via guard (automatic on panic)
let _cleanup = CleanupGuard::new(runner.clone());
runner.run(&mut scenario).await?;
```

**DON'T: Mix concerns in one scenario**
```rust,ignore
// BAD: Hard to debug when it fails
.transactions_with(|tx| tx.rate(50).users(100))  // high load
.chaos_with(|c| c.restart().min_delay(...))        // AND chaos
.da_with(|da| da.channel_rate(10).blob_rate(20))  // AND DA stress

// GOOD: Separate tests for each concern
// Test 1: High transaction load only
// Test 2: Chaos resilience only
// Test 3: DA stress only
```

**DON'T: Hardcode paths or ports**
```rust,ignore
// BAD: Breaks on different machines
let path = PathBuf::from("/home/user/circuits/kzgrs_test_params");
let port = 9000; // might conflict

// GOOD: Use env vars and dynamic allocation
let path = std::env::var("NOMOS_KZGRS_PARAMS_PATH")
    .unwrap_or_else(|_| "assets/stack/kzgrs_test_params/kzgrs_test_params".to_string());
let port = get_available_tcp_port();
```

**DON'T: Ignore resource limits**
```bash
# BAD: Large topology without checking resources
scripts/run/run-examples.sh -v 20 -e 10 compose
# (might OOM or exhaust ulimits)

# GOOD: Scale gradually and monitor resources
scripts/run/run-examples.sh -v 3 -e 2 compose  # start small
docker stats  # monitor resource usage
# then increase if resources allow
```

## Scenario Design Heuristics

**Minimal viable topology**
- Consensus: 3 validators (minimum for Byzantine fault tolerance)
- Network: Star topology (simplest for debugging)

**Workload rate selection**
- Start with 1-5 tx/s per user, then increase
- DA: 1-2 channels, 1-3 blobs/channel initially
- Chaos: 30s+ intervals between restarts (allow recovery)

**Duration guidelines**

| Test Type | Minimum Duration | Typical Duration |
|-----------|------------------|------------------|
| Smoke test | 30s | 60s |
| Integration test | 60s | 120s |
| Load test | 120s | 300s |
| Resilience test | 120s | 300s |
| Soak test | 600s (10m) | 3600s (1h) |

**Expectation selection**

| Test Goal | Expectations |
|-----------|--------------|
| Basic functionality | `expect_consensus_liveness()` |
| Transaction handling | `expect_consensus_liveness()` + custom inclusion check |
| DA correctness | `expect_consensus_liveness()` + DA dispersal/sampling checks |
| Resilience | `expect_consensus_liveness()` + recovery time measurement |

## Testing the Tests

**Validate scenarios before committing**
1. Run on Host runner first (fast feedback)
2. Run on Compose runner (reproducibility check)
3. Check logs for warnings or errors
4. Verify cleanup (no leaked processes/containers)
5. Run 2-3 times to check for flakiness

**Handling flaky tests**
- Increase run duration (timing-sensitive assertions need longer runs)
- Reduce workload rates (might be saturating nodes)
- Check resource limits (CPU/RAM/ulimits)
- Add debugging output to identify race conditions
- Consider if test is over-specified (too strict expectations)

**See also:**
- [Troubleshooting](troubleshooting.md) for common failure patterns
- [FAQ](faq.md) for design decisions and gotchas
