# Running Scenarios

Running a scenario follows the same conceptual flow regardless of environment:

1. Select or author a scenario plan that pairs a topology with workloads,
   expectations, and a suitable run window.
2. Choose a runner aligned with your environment (local, compose, or k8s) and
   ensure its prerequisites are available.
3. Deploy the plan through the runner; wait for readiness signals before
   starting workloads.
4. Let workloads drive activity for the planned duration; keep observability
   signals visible so you can correlate outcomes.
5. Evaluate expectations and capture results as the primary pass/fail signal.

Use the same plan across different runners to compare behavior between local
development and CI or cluster settings. For environment prerequisites and
flags, see [Operations](operations.md).
