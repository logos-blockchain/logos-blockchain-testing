# Logos Testing Framework

A Rust framework for system-level tests of networked applications. It can start
local processes and node clusters, deploy uniform clusters with Docker Compose
or Kubernetes, connect to existing deployments, run test activity, evaluate
outcomes, and clean up the resources it started.

The framework is application-agnostic. Application repositories provide their
node configuration, clients, readiness checks, and backend-specific launch
settings.

## Start Here

The workspace pins its Rust toolchain in `rust-toolchain.toml`. The local
kvstore example needs no additional setup; its node binary is built on first
use:

```bash
cargo run -p kvstore-examples --bin kvstore_app_host_convergence
```

This starts three local node processes, writes data, restarts a node, checks
the result, and removes the processes and temporary directories.

The composed-application example runs as integration tests:

```bash
cargo test -p multi-app-e2e
```

It covers a queue cluster, worker process, and result-store cluster through a
runner-driven scenario and direct imperative tests.

For Compose examples, run a Docker daemon and prepare the image named by the
example; the Compose deployer checks that it exists locally but does not build
or pull it. Kubernetes examples require a reachable cluster, `kubectl`, Helm,
and a node image available to that cluster.

See [Quickstart](book/src/quickstart.md) and
[Running the Examples](book/src/running-examples.md) for the complete commands
and requirements.

## Ways to Write Tests

### Scenarios

A scenario records the system to deploy, workloads to run, expectations to
evaluate, runtime limits, and enabled capabilities. The runner performs
deployment, readiness checks, concurrent workloads, cooldown, expectation
evaluation, and cleanup.

```rust
let mut scenario = AppHost::scenario()
    .with_app(KvLocalApp::nodes(3))
    .with_workload(KvAppHostConvergence::new(3))
    .with_run_duration(Duration::from_secs(5))
    .build()?;

let runner = AppHostLocalDeployer::default()
    .deploy(&scenario)
    .await?;

runner.run(&mut scenario).await?;
```

### Imperative Tests

`ManualCluster` gives ordinary Rust or a BDD harness direct control of one
uniform cluster. Tests can start, stop, restart, and wait for nodes without
using workloads or expectations.

A composed application can also be deployed directly through `DeployContext`
when test code needs to control the complete stack step by step.

### Composed Applications

`AppDeployment` describes how application components are started and connected.
`AppHost` runs one root deployment as part of a scenario. Child deployments can
start uniform clusters through `LocalAppCluster` and standalone binaries through
`LocalProcessApp`, then expose typed handles to workloads and expectations.

App composition currently runs only with the local process deployer. Compose
and Kubernetes support uniform application clusters, not an `AppDeployment`
tree containing several application types.

### Existing Deployments

Scenarios can use managed nodes, attach to an existing Compose project or
Kubernetes deployment, or construct clients for external endpoints. Available
node control depends on the selected source and backend.

## Deployment Backends

Uniform scenarios use the same scenario runtime on all three backends. Each
application supplies a thin backend adapter containing details such as the
binary or image, config location, and service ports.

| Capability | Local | Compose | Kubernetes |
|---|---|---|---|
| Uniform managed scenarios | Yes | Yes | Yes |
| Managed node control | Start, stop, restart | Restart | Use Kubernetes `ManualCluster` |
| Existing clusters | No | Compose project or services | Label selector and namespace |
| External endpoints | Yes | Yes | Yes |
| `AppHost` composition | Yes | No | No |
| Config delivery | Files in node working directories | cfgsync | cfgsync |

The local deployer resolves executable paths through path, environment, build,
or download providers. Compose generates a project and services. Kubernetes
installs a Helm release in a per-run namespace. Container backends deliver
generated per-node configuration and other static files through cfgsync.

See the [Capability Matrix](book/src/capability-matrix.md),
[Local Deployer](book/src/deployer-local.md),
[Compose Deployer](book/src/deployer-compose.md), and
[Kubernetes Deployer](book/src/deployer-k8s.md).

## Repository Layout

```text
testing-framework/
├── core/                 scenario runtime, topology, provisioning, control
├── app/                  AppHost, AppDeployment, typed handles, composition
└── deployers/
    ├── local/            local processes and binary providers
    ├── compose/          generated Docker Compose projects
    └── k8s/              Helm and Kubernetes deployment

cfgsync/
├── artifacts/            backend-neutral per-node files
├── core/                 protocol, server, client, and rendering
├── adapter/              application config materialization
└── runtime/              cfgsync server and client binaries

examples/                 self-contained example applications and tests
book/                     mdBook source and presentation theme
scripts/                  checks, cleanup, and observability helpers
```

The example applications include uniform clusters, composed stacks, consensus
failover, queues, WebSocket pub/sub, metrics, and unmodified NATS and Redis
servers. See [examples/README.md](examples/README.md) for the recommended entry
points.

## Documentation

- [The Framework in Brief](book/src/framework-in-brief.md)
- [Quickstart](book/src/quickstart.md)
- [Application and Environment Model](book/src/application-model.md)
- [Composing Applications](book/src/part-ii.md)
- [Scenario Runtime](book/src/part-iii.md)
- [Uniform Clusters and Configuration](book/src/part-iv.md)
- [Deployers and Sources](book/src/part-v.md)
- [Environment Variables](book/src/environment-variables.md)
- [Troubleshooting](book/src/troubleshooting.md)

Published book: <https://logos-blockchain.github.io/logos-blockchain-testing/>

Build and test it locally with:

```bash
mdbook build book
mdbook test book
```

Install `mdbook` first if it is not already available.

## Development

Useful focused checks from the workspace root:

```bash
cargo fmt --all -- --check
cargo test -p testing-framework-core
cargo test -p testing-framework-app
cargo test -p multi-app-e2e
cargo clippy --all --all-targets --all-features -- -D warnings
```

The lint workflow also checks dependency policy with `cargo-deny`, unused
dependencies with `cargo-machete`, and TOML formatting with Taplo.

## License

MIT OR Apache-2.0.
