# FAQ

**Why block-oriented timing?**  
Using block cadence reduces dependence on host speed and keeps assertions aligned
with protocol behavior.

**Can I reuse the same scenario across runners?**  
Yes. The plan stays the same; swap runners (local, compose, k8s) to target
different environments.

**When should I enable chaos workloads?**  
Only when testing resilience or operational recovery; keep functional smoke
tests deterministic.

**How long should runs be?**  
Long enough for multiple blocks so liveness and inclusion checks are
meaningful; very short runs risk false confidence.

**Do I always need seeded wallets?**  
Only for transaction scenarios. Data-availability or pure chaos scenarios may
not require them, but liveness checks still need validators producing blocks.

**What if expectations fail but workloads “look fine”?**  
Trust expectations first—they capture the intended success criteria. Use the
observability signals and runner logs to pinpoint why the system missed the
target.
