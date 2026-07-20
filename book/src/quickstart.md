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
cargo run -p kvstore-examples --bin kvstore_app_host_convergence
```

**First run takes a few minutes** (builds the framework and the `kvstore-node` binary).

**What happens:**

1. `AppHost::scenario()` builds a scenario around a composed application instead of a managed node topology.
2. `with_app(KvLocalApp::nodes(3))` deploys a three-node kvstore cluster as local processes.
3. The convergence workload writes a value, restarts `node-0`, waits for readiness, and writes again.
4. The runner evaluates the outcome and tears the cluster down.

**What you should see:**

- Three `kvstore-node` processes spawn with generated configs in per-run temporary directories
- The workload logs a successful write before and after the restart
- The command exits successfully and removes the temporary directories

---

## The Code Behind It

The binary is short enough to read in full at `examples/kvstore/examples/src/bin/app_host_convergence.rs`. Its core is:

```rust,ignore
let mut scenario = AppHost::scenario()
    .with_app(KvLocalApp::nodes(3))
    .with_run_duration(Duration::from_secs(5))
    .with_workload(KvAppHostConvergence::new(3))
    .build()?;

let deployer = AppHostLocalDeployer::default();
let runner = deployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

The workload reaches the deployed cluster through a typed handle (`RunContext` is the object every workload receives at run time; see [Part III](part-iii.md)):

```rust,ignore
async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
    let cluster = ctx.require_app::<LocalAppCluster<KvEnv>>()?;

    put_value(&cluster, "before-restart").await?;
    cluster.restart_node("node-0").await?;
    cluster.wait_node_ready("node-0").await?;
    put_value(&cluster, "after-restart").await?;
    Ok(())
}
```

The same pattern can run in `#[tokio::test]` functions. The composition acceptance suite does this:

```bash
cargo test -p multi-app-e2e
```

It uses a reusable fixture crate for the stack, workload, and expectation, then drives them from ordinary integration tests.

---

## Where to Go Next

| Goal | Read |
|------|------|
| Understand the abstractions you just used | [Part I — Mental Model](part-i.md) |
| Compose your own application stack | [Part II — Composing Applications](part-ii.md) |
| Write workloads and expectations | [Part III — Scenario Runtime](part-iii.md) |
| Put your own node behind the framework | [Part IV — Uniform Clusters](part-iv.md) |
| Run against Compose, Kubernetes, or a live network | [Part V — Deployers and Sources](part-v.md) |
