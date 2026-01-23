# Topology & Chaos Patterns

This page focuses on cluster manipulation: node control, chaos patterns, and
what the tooling supports today.

## Node control availability
- **Supported**: restart control via `NodeControlHandle` (compose runner).
- **Not supported**: local runner does not expose node control; k8s runner does
  not support it yet.
- **Not yet supported**: peer blocking/unblocking and network partitions.

See also: [RunContext: BlockFeed & Node Control](node-control.md) for the current node-control API surface and limitations.

## Chaos patterns to consider
- **Restarts**: random restarts with minimum delay/cooldown to test recovery.
- **Partitions (planned)**: block/unblock peers to simulate partial isolation, then assert
  height convergence after healing.
- **Validator churn (planned)**: stop one validator and start another (new key) mid-run to
  test membership changes; expect convergence.
- **Load SLOs**: push tx/DA rates and assert inclusion/availability budgets
  instead of only liveness.
- **API probes**: poll HTTP/RPC endpoints during chaos to ensure external
  contracts stay healthy (shape + latency).

## Expectations to pair
- **Liveness/height convergence** after chaos windows.
- **SLO checks**: inclusion latency, DA responsiveness, API latency/shape.
- **Recovery checks**: ensure nodes that were isolated or restarted catch up to
  cluster height within a timeout.

## Guidance
- Keep chaos realistic: avoid flapping or patterns you wouldn't operate in prod.
- Scope chaos: choose validators intentionally; don't restart all
  nodes at once unless you're testing full outages.
- Combine chaos with observability: capture block feed/metrics and API health so
  failures are diagnosable.
