# Testing Philosophy

- **Declarative over imperative**: describe the desired cluster shape, traffic, and success criteria; let the framework orchestrate the run.
- **Observable health signals**: prefer liveness and inclusion signals that reflect real user impact instead of internal debug state.
- **Determinism first**: default scenarios aim for repeatable outcomes with fixed topologies and traffic rates; variability is opt-in.
- **Targeted non-determinism**: introduce randomness (e.g., restarts) only when probing resilience or operational robustness.
- **Protocol time, not wall time**: reason in blocks and protocol-driven intervals to reduce dependence on host speed or scheduler noise.
- **Minimum run window**: always allow enough block production to make assertions meaningful; very short runs risk false confidence.
- **Use chaos with intent**: chaos workloads are for recovery and fault-tolerance validation, not for baseline functional checks.
