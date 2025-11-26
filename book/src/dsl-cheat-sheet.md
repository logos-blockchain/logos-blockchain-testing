# Core Content: DSL Cheat Sheet

The framework offers a fluent builder style to keep scenarios readable. Common
knobs:

- **Topology shaping**: set validator and executor counts, pick a network layout
  style, and adjust high-level data-availability traits.
- **Wallet seeding**: define how many users participate and the total funds
  available for transaction workloads.
- **Workload tuning**: configure transaction rates, data-availability channel
  and blob rates, and whether chaos restarts should include validators,
  executors, or both.
- **Expectations**: attach liveness and workload-specific checks so success is
  explicit.
- **Run window**: set a minimum duration long enough for multiple blocks to be
  observed and verified.

Use these knobs to express intent clearly, keeping scenario definitions concise
and consistent across teams.
