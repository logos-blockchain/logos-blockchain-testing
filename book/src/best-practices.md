# Best Practices

- **State your intent**: document the goal of each scenario (throughput, DA
  validation, resilience) so expectation choices are obvious.
- **Keep runs meaningful**: choose durations that allow multiple blocks and make
  timing-based assertions trustworthy.
- **Separate concerns**: start with deterministic workloads for functional
  checks; add chaos in dedicated resilience scenarios to avoid noisy failures.
- **Reuse patterns**: standardize on shared topology and workload presets so
  results are comparable across environments and teams.
- **Observe first, tune second**: rely on liveness and inclusion signals to
  interpret outcomes before tweaking rates or topology.
- **Environment fit**: pick runners that match the feedback loop you need—local
  for speed, compose for reproducible stacks, k8s for cluster-grade fidelity.
- **Minimal surprises**: seed only necessary wallets and keep configuration
  deltas explicit when moving between CI and developer machines.
