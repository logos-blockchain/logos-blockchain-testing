# Seeds and Reproducibility

`DeploymentSeed` controls only part of a run's variability. This chapter lists what it seeds, what it does not, and what that means for reproducing a run.

---

## DeploymentSeed

`DeploymentSeed` (`testing-framework/core/src/topology/mod.rs`) is a 32-byte value:

```rust,ignore
let seed = DeploymentSeed::new([7u8; 32]);
let bytes: &[u8; 32] = seed.bytes();
```

The seed exists so generated deployments can be reproduced: record it when a run fails, and the same seed makes the provider return the identical deployment.

You attach it to a scenario with `with_deployment_seed`:

```rust,ignore
let scenario = ScenarioBuilder::<KvEnv>::new(provider)
    .with_deployment_seed(DeploymentSeed::new([7u8; 32]))
    .with_run_duration(Duration::from_secs(30))
    .build()?;
```

The seed has exactly one consumer: when `build()` resolves the deployment, it calls the deployment provider with it:

```rust,ignore
pub trait DeploymentProvider<D>: Send + Sync {
    fn build(&self, seed: Option<&DeploymentSeed>) -> Result<D, DynTopologyError>;
}
```

A provider that generates topologies (random shapes, sampled node parameters, derived node ids) should draw all of its randomness from the seed, so the same seed always yields the same deployment. See [Topology and Deployment Plans](topology.md) for how providers feed the builder.

---

## What Is Actually Seeded

The current behavior is:

| Concern | Seeded? |
|---|---|
| Deployment generation by a *custom* `DeploymentProvider` | Yes — the seed is passed to `build()` |
| `FixedDeploymentProvider` (the `with_deployment(...)` path) | No — the seed is accepted and ignored |
| Local port assignment | No — ports come from the OS (`bind 127.0.0.1:0`; see [node-config.md](node-config.md)) |
| Node working directories | No — temp directories get random suffixes (see [Persistence](persistence.md)) |
| Workload timing, scheduling, network behavior | No |

No in-repo deployment provider currently consumes the seed: `FixedDeploymentProvider` is the only provider shipped, and the example apps all use concrete `ClusterTopology` values. `DeploymentSeed` is available to custom generating providers; setting a seed on a fixed deployment has no effect.

---

## Writing a Seeded Provider

A provider that wants reproducible generation reads all of its variability from the seed bytes:

```rust,ignore
use testing_framework_core::topology::{
    ClusterTopology, DeploymentProvider, DeploymentSeed, DynTopologyError,
};

struct SizedFromSeed {
    min_nodes: usize,
    max_nodes: usize,
}

impl DeploymentProvider<ClusterTopology> for SizedFromSeed {
    fn build(&self, seed: Option<&DeploymentSeed>) -> Result<ClusterTopology, DynTopologyError> {
        let first = seed.map_or(0, |seed| seed.bytes()[0] as usize);
        let span = self.max_nodes - self.min_nodes + 1;
        Ok(ClusterTopology::new(self.min_nodes + first % span))
    }
}

let scenario = ScenarioBuilder::<KvEnv>::new(Box::new(SizedFromSeed {
        min_nodes: 3,
        max_nodes: 7,
    }))
    .with_deployment_seed(DeploymentSeed::new([7u8; 32]))
    .with_run_duration(Duration::from_secs(30))
    .build()?;
```

The same seed always produces the same cluster size; omitting the seed uses the provider's default (`seed` is `None`). A generating provider can feed the 32 bytes into a seeded RNG and derive node parameters or `NodePlan` ids from it.

---

## Practical Reproducibility

- If your provider is seeded, record the seed alongside failures and replay with `with_deployment_seed` to get the identical deployment.
- Everything downstream of the deployment (ports, PIDs, timing) still varies run to run. Determinism ends at the descriptor; treat expectations accordingly.
- For state-level reproduction (replaying a node from captured state rather than regenerating a topology), use snapshot directories instead; see [Persistence, Snapshots, and Recovery Testing](persistence.md).
