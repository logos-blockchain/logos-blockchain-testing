# Quickstart

Run a complete multi-node test in one command.

---

## Prerequisites

- Rust toolchain (the workspace pins its version via `rust-toolchain.toml`)
- Unix-like system (tested on Linux and macOS)
- For Compose examples: a running Docker daemon
- For Kubernetes examples: a reachable cluster context

No other setup. Example node binaries are resolved automatically; the kvstore example builds its node with Cargo on first run if no prebuilt binary is available.

---

## Your First Test

```bash
git clone <this-repository>
cd <this-repository>
cargo run -p kvstore-examples --bin kvstore_basic_convergence
```

**First run takes a few minutes** (builds the framework and the `kvstore-node` binary).

**What happens:**

1. `AppHost::scenario()` creates an empty scenario that gets its whole system from application deployments.
2. `with_app(ClusterApp::<KvEnv>::new(KvTopology::new(3)))` deploys a three-node kvstore cluster as local processes through the default `LocalClusterProvisioner`.
3. The write workload streams 300 keyed writes into the cluster while it runs.
4. The convergence expectation polls every node until all values agree, then the runner tears the cluster down.

**What you should see:**

- Three `kvstore-node` processes spawn with generated configs in per-run temporary directories
- The workload logs its write progress; the expectation reports convergence
- The command exits successfully and removes the temporary directories

---

## The Code Behind It

The binary is short enough to read in full at `examples/kvstore/examples/src/bin/basic_convergence.rs`. Its core is:

```rust,ignore
let mut scenario = AppHost::scenario()
    .with_app(ClusterApp::<KvEnv>::new(KvTopology::new(3)))
    .with_run_duration(Duration::from_secs(30))
    .with_workload(
        KvWriteWorkload::new()
            .operations(300)
            .key_count(30)
            .rate_per_sec(30)
            .key_prefix("demo"),
    )
    .with_expectation(KvConverges::new("demo", 30).timeout(Duration::from_secs(25)))
    .build()?;

let runner = AppHostDeployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

The workload reaches the deployed cluster through a typed handle (`RunContext` is the object every workload receives at run time; see [Part III](part-iii.md)):

```rust,ignore
async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
    let cluster = ctx.require_app::<ClusterHandle<KvEnv>>()?;
    let client = cluster.first_client().ok_or("kv cluster has no clients")?;

    put_value(&client, "demo-0", "value-0").await?;
    Ok(())
}
```

`ClusterHandle` is backend-neutral: swapping `with_app` for `with_app_using(..., ComposeProvisioner::default())` or `with_app_using(..., K8sClusterProvisioner)` runs the same scenario on Docker Compose or Kubernetes without touching the workload.

The same pattern also runs in `#[tokio::test]` functions. The kvstore smoke test and the composition acceptance suite do this:

```bash
cargo test -p kvstore-examples --test local_smoke
cargo test -p multi-app-e2e
```

The latter uses a reusable fixture crate for a multi-application stack, workload, and expectation, then drives them from ordinary integration tests.

---

## Where to Go Next

| Goal | Read |
|------|------|
| Understand the abstractions you just used | [Part I — Mental Model](part-i.md) |
| Compose your own application stack | [Part II — Composing Applications](part-ii.md) |
| Write workloads and expectations | [Part III — Scenario Runtime](part-iii.md) |
| Put your own node behind the framework | [Part IV — Uniform Clusters](part-iv.md) |
| Run against Compose, Kubernetes, or a live network | [Part V — Backends and Sources](part-v.md) |
