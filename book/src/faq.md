# FAQ

**Why block-oriented timing?**  
Slots advance at a fixed rate (NTP-synchronized, 2s by default), so reasoning
about blocks and consensus intervals keeps assertions aligned with protocol
behavior rather than arbitrary wall-clock durations.

**Can I reuse the same scenario across runners?**  
Yes. The plan stays the same; swap runners (local, compose, k8s) to target
different environments.

**When should I enable chaos workloads?**  
Only when testing resilience or operational recovery; keep functional smoke
tests deterministic.

**How long should runs be?**  
The framework enforces a minimum of **2× slot duration** (4 seconds with default 2s slots), but practical recommendations:

- **Smoke tests**: 30s minimum (~14 blocks with default 2s slots, 0.9 coefficient)
- **Transaction workloads**: 60s+ (~27 blocks) to observe inclusion patterns
- **Chaos tests**: 120s+ (~54 blocks) to allow recovery after restarts

Very short runs (< 30s) risk false confidence—one or two lucky blocks don't prove liveness.

**Do I always need seeded wallets?**  
Only for transaction scenarios. Pure chaos scenarios may not require them, but
liveness checks still need nodes producing blocks.

**What if expectations fail but workloads “look fine”?**  
Trust expectations first—they capture the intended success criteria. Use the
observability signals and runner logs to pinpoint why the system missed the
target.
