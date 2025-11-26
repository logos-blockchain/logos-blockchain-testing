# Authoring Scenarios

Creating a scenario is a declarative exercise:

1. **Shape the topology**: decide how many validators and executors to run, and
   what high-level network and data-availability characteristics matter for the
   test.
2. **Attach workloads**: pick traffic generators that align with your goals
   (transactions, data-availability blobs, or chaos for resilience probes).
3. **Define expectations**: specify the health signals that must hold when the
   run finishes (e.g., consensus liveness, inclusion of submitted activity; see
   [Core Content: Workloads & Expectations](workloads.md)).
4. **Set duration**: choose a run window long enough to observe meaningful
   block progression and the effects of your workloads.
5. **Choose a runner**: target local processes for fast iteration, Docker
   Compose for reproducible multi-node stacks, or Kubernetes for cluster-grade
   validation. For environment considerations, see [Operations](operations.md).

Keep scenarios small and explicit: make the intended behavior and the success
criteria clear so failures are easy to interpret and act upon.
