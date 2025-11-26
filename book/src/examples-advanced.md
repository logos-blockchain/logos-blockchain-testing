# Advanced & Artificial Examples

These illustrative scenarios stretch the framework to show how to build new
workloads, expectations, deployers, and topology tricks. They are intentionally
“synthetic” to teach capabilities rather than prescribe production tests.

## Synthetic Delay Workload (Network Latency Simulation)
- **Idea**: inject fake latency between node interactions using internal timers,
  not OS-level tooling.
- **Demonstrates**: sequencing control inside a workload, verifying protocol
  progression under induced lag, using timers to pace submissions.
- **Shape**: wrap submissions in delays that mimic slow peers; ensure the
  expectation checks blocks still progress.

## Oscillating Load Workload (Traffic Waves)
- **Idea**: traffic rate changes every block or N seconds (e.g., blocks 1–3 low,
  4–5 high, 6–7 zero, repeat).
- **Demonstrates**: dynamic, stateful workloads that use `RunMetrics` to time
  phases; modeling real-world burstiness.
- **Shape**: schedule per-phase rates; confirm inclusion/liveness across peaks
  and troughs.

## Byzantine Behavior Mock
- **Idea**: a workload that drops half its planned submissions, sometimes
  double-submits, and intentionally triggers expectation failures.
- **Demonstrates**: negative testing, resilience checks, and the value of clear
  expectations when behavior is adversarial by design.
- **Shape**: parameterize drop/double-submit probabilities; pair with an
  expectation that documents what “bad” looks like.

## Custom Expectation: Block Finality Drift
- **Idea**: assert the last few blocks differ and block time stays within a
  tolerated drift budget.
- **Demonstrates**: consuming `BlockFeed` or time-series metrics to validate
  protocol cadence; crafting post-run assertions around block diversity and
  timing.
- **Shape**: collect recent blocks, confirm no duplicates, and compare observed
  intervals to a drift threshold.

## Custom Deployer: Dry-Run Deployer
- **Idea**: a deployer that never starts nodes; it emits configs, simulates
  readiness, provides fake blockfeed/metrics.
- **Demonstrates**: full power of the deployer interface for CI dry-runs,
  config verification, and ultra-fast feedback without Nomos binaries.
- **Shape**: produce logs/artifacts, stub readiness, and feed synthetic blocks
  so expectations can still run.

## Stochastic Topology Generator
- **Idea**: topology parameters change at runtime (random validators, DA
  settings, network shapes).
- **Demonstrates**: randomized property testing and fuzzing approaches to
  topology building.
- **Shape**: pick roles and network layouts randomly per run; keep expectations
  tolerant to variability while still asserting core liveness.

## Multi-Phase Scenario (“Pipelines”)
- **Idea**: scenario runs in phases (e.g., phase 1 transactions, phase 2 DA,
  phase 3 restarts, phase 4 sync check).
- **Demonstrates**: multi-stage tests, modular scenario assembly, and deliberate
  lifecycle control.
- **Shape**: drive phase-specific workloads/expectations sequentially; enforce
  clear boundaries and post-phase checks.
