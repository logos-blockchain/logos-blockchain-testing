# One Binary: LocalProcessApp

`LocalProcessApp<C>` deploys one local binary with a typed client, without modeling it as a node topology.

Application code supplies the launch files, client type, and readiness check. The framework manages the process lifetime, working directory, and teardown. This is used for third-party infrastructure such as a message broker or database, and for singleton services such as a sequencer or indexer inside a composed stack.

---

## Construction

```rust,ignore
LocalProcessApp::new(label, launch, endpoints, client)
```

| Argument | Type | Meaning |
|----------|------|---------|
| `label` | `impl Into<String>` | Name used for the process working directory and logs. |
| `launch` | `LaunchSpec` | How to start the binary. |
| `endpoints` | `NodeEndpoints` | Addresses the process will listen on. |
| `client` | `C: Clone + Send + Sync + 'static` | The typed client returned through the handle. |

`LaunchSpec` (from `testing_framework_runner_local`) is a plain launch plan:

| Field | Type | Purpose |
|-------|------|---------|
| `binary` | `PathBuf` | Executable path. |
| `files` | `Vec<LaunchFile>` | Files written into the working directory before spawn (`relative_path` + `contents`). |
| `args` | `Vec<String>` | Command-line arguments. |
| `env` | `Vec<LaunchEnvVar>` | Environment variables (`LaunchEnvVar::new(key, value)`). |

`NodeEndpoints` describes where the process listens: an `api: SocketAddr` plus `extra_ports` keyed by `NodeEndpointPort` (`TestingApi`, `Network`, or `Custom(String)`). Build one with `NodeEndpoints::from_api_port(port)` and `insert_port`.

Endpoints are *declared*, not allocated. The generic process layer does not select ports. The launch configuration and the supplied endpoints must use the same values.

---

## Builder Options

| Method | Effect |
|--------|--------|
| `with_readiness(closure)` | Async check run after spawn. The closure receives `(NodeEndpoints, C)`. **On failure the process is stopped and the deploy fails.** |
| `keep_tempdir(bool)` | Keep the generated working directory after teardown. |
| `with_persist_dir(path)` | Place the working directory next to `path` (as `<basename>_<random>`); nothing is copied — see [Persistence](persistence.md). |
| `with_snapshot_dir(path)` | Copy the snapshot directory's contents into the fresh working directory before start. |

If the readiness closure fails, deployment returns an error, stops the new process, and cleans up children deployed earlier (see [Handle Ownership and Teardown](handles-teardown.md)).

---

## Example: A Single nats-server Process

The nats example normally runs as a uniform cluster, but its `NatsClient` works just as well against one broker started as a process app:

```rust,ignore
use std::time::Duration;

use nats_runtime_ext::NatsClient;
use testing_framework_app::LocalProcessApp;
use testing_framework_runner_local::{LaunchSpec, NodeEndpoints};

let launch = LaunchSpec {
    binary: std::env::var("NATS_SERVER_BIN")?.into(),
    args: vec!["-p".into(), "4222".into(), "-m".into(), "8222".into()],
    ..LaunchSpec::default()
};

let client = NatsClient::new(
    "nats://127.0.0.1:4222".to_owned(),
    "http://127.0.0.1:8222".parse()?,
);

let broker = LocalProcessApp::new("nats", launch, NodeEndpoints::from_api_port(8222), client)
    .with_readiness(|_endpoints, client| async move {
        for _ in 0..50 {
            if client.is_healthy().await.unwrap_or(false) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("nats-server did not become healthy".into())
    });
```

`broker` is an `AppDeployment` for any environment, so a root deployment composes it like any other child:

```rust,ignore
let nats = ctx.deploy_and_expose(broker).await?;
```

---

## The Handle: LocalProcessHandle

`deploy` returns `LocalProcessHandle<C>`. Clones share access to the same process state, while scenario cleanup owns the process lifetime. Cleanup stops the process even if a handle clone still exists.

<details>
<summary>LocalProcessHandle method reference</summary>

| Method | Returns | Notes |
|--------|---------|-------|
| `client()` | `C` | Clone of the typed client. |
| `endpoints()` | `&NodeEndpoints` | The endpoints supplied at deployment. |
| `pid()` | `u32` | OS process id (async). |
| `is_running()` | `bool` | Whether the child is still alive (async). |
| `working_dir()` | `PathBuf` | The generated working directory (async). |
| `start()` | `Result<(), DynError>` | Start again after an explicit stop, using the original `LaunchSpec`. |
| `restart()` | `Result<(), DynError>` | Restart with the original `LaunchSpec`. |
| `stop()` | — | Stop now, without waiting for drop. |
| `keep_tempdir()` | `io::Result<()>` | Retain the working directory at teardown. |

</details>

A workload retrieves the handle like any other (see [AppHost and with_app](app-host.md)):

```rust,ignore
let broker = ctx.require_app::<LocalProcessHandle<NatsClient>>()?;
broker.restart().await?;
assert!(broker.is_running().await);
```

Tests can use `start`, `restart`, and `stop` for process lifecycle and fault injection. These operations fail after scenario cleanup has closed the managed resource.

---

## See Also

- [AppDeployment and DeployContext](app-deployment.md): composing a process app under a root deployment.
- [Composing Heterogeneous Stacks](composing-stacks.md): mixing single processes with child clusters.
