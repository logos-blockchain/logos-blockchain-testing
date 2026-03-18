# cfgsync

`cfgsync` is a small library stack for bootstrap-time config delivery.

The library solves one problem: nodes need to identify themselves, wait until configuration is ready, fetch the files they need, write them locally, and then continue startup. `cfgsync` owns that transport and serving loop. The application using it still decides what “ready” means and what files should be generated.

That split is the point of the design:

- `cfgsync` owns registration, polling, payload transport, and file delivery.
- the application adapter owns readiness policy and artifact generation.

The result is a reusable library without application-specific bootstrap logic leaking into core crates.

## How it works

The normal flow is registration-backed serving.

Each node first sends a registration containing:

- a stable node identifier
- its IP address
- optional typed application metadata

The server stores registrations and builds a `RegistrationSnapshot`. The application provides a `RegistrationSnapshotMaterializer`, which receives that snapshot and decides whether configuration is ready yet.

If the materializer returns `NotReady`, the node keeps polling. If it returns `Ready`, cfgsync serves one payload containing:

- node-local files for the requesting node
- optional shared files that every node should receive

The node then writes those files locally and continues startup.

That is the main model. Everything else is a variation of it.

## Precomputed artifacts

Some systems already know the final artifacts before any node starts. That still fits the same model.

In that case the server simply starts with precomputed `MaterializedArtifacts`. Nodes still register and fetch through the same protocol, but the materializer already knows the final outputs. Registration becomes an identity and readiness gate, not a source of topology discovery.

This is why cfgsync no longer needs a separate “static mode” as a first-class concept. Precomputed serving is just registration-backed serving with an already-known result.

## Crate layout

### `cfgsync-artifacts`

This crate contains the file-level data model:

- `ArtifactFile` for a single file
- `ArtifactSet` for a group of files

If all you need is “what files exist and how are they grouped”, this is the crate to look at.

### `cfgsync-core`

This crate contains the protocol and the low-level HTTP implementation.

Important types here are:

- `NodeRegistration`
- `RegistrationPayload`
- `NodeArtifactsPayload`
- `Client`
- `NodeConfigSource`

It also defines the HTTP contract:

- `POST /register`
- `POST /node`

The server answers with either a payload, `NotReady`, or `Missing`.

### `cfgsync-adapter`

This crate defines the application-facing seam.

The key types are:

- `RegistrationSnapshot`
- `RegistrationSnapshotMaterializer`
- `MaterializedArtifacts`
- `MaterializationResult`

The adapter’s job is simple: given the current registration snapshot, decide whether artifacts are ready, and if they are, return them.

The crate also contains reusable wrappers around that seam:

- `CachedSnapshotMaterializer`
- `PersistingSnapshotMaterializer`
- `RegistrationConfigSource`

These exist because caching and result persistence are generic orchestration concerns, not application-specific logic.

### `cfgsync-runtime`

This crate provides the operational entrypoints.

Use it when you want to run cfgsync rather than define its protocol:

- client-side fetch/write helpers
- server config loading
- direct serving helpers such as `serve(...)`

This is the crate that should feel like the normal “start here” path for users integrating cfgsync into a real system.

## Artifact model

The adapter usually thinks in full snapshots, but cfgsync serves one node at a time.

The materializer returns `MaterializedArtifacts`, which contain:

- node-local artifacts keyed by node identifier
- optional shared artifacts

When one node fetches config, cfgsync resolves that node’s local files, merges in the shared files, and returns a single payload.

That is why applications usually do not need a second “shared config” endpoint. Shared files can travel in the same payload as node-local files.

## The adapter boundary

The adapter is where application semantics belong.

In practice, the adapter should define:

- the typed registration payload
- the readiness rule
- the conversion from registration snapshots into artifacts
- any shared artifact generation the application needs

Typical examples are:

- waiting for `n` initial nodes
- deriving peer lists from registrations
- generating one node-local config file per node
- generating one shared deployment file for all nodes

What does not belong in cfgsync core is equally important. Generic cfgsync should not understand:

- application-specific topology semantics
- genesis or deployment generation rules for one protocol
- application-specific command/state-machine logic
- domain-specific ideas of what a node “really is”

Those belong in the adapter or in the consuming application.

## Start here

Start with the examples in `cfgsync/runtime/examples/`.

- `minimal_cfgsync.rs` shows the smallest complete flow: serve cfgsync, register
  one node, fetch artifacts, and write them locally.
- `precomputed_registration_cfgsync.rs` shows how precomputed artifacts still
  use the same registration flow, including a later node that joins after the
  server is already running.
- `wait_for_registrations_cfgsync.rs` shows the normal `NotReady` path: one node
  waits until the materializer sees enough registrations, then both nodes
  receive config.

Those three examples cover the full public model. The rest of this README just
names the pieces and explains where application-specific logic belongs.

## Minimal integration path

For a new application, the shortest sensible path is:

1. define a typed registration payload
2. implement `RegistrationSnapshotMaterializer`
3. return node-local and optional shared artifacts
4. serve them with `serve(...)`
5. use `Client` on the node side

That gives you the main value of the library without pushing application logic
into cfgsync itself.

## API sketch

Typed registration payload:

```rust
use cfgsync_core::NodeRegistration;

#[derive(serde::Serialize)]
struct MyNodeMetadata {
    network_port: u16,
    api_port: u16,
}

let registration = NodeRegistration::new("node-1".to_string(), "127.0.0.1".parse().unwrap())
    .with_metadata(&MyNodeMetadata {
        network_port: 3000,
        api_port: 18080,
    })?;
```

Snapshot materializer:

```rust
use cfgsync_adapter::{
    DynCfgsyncError, MaterializationResult, MaterializedArtifacts, RegistrationSnapshot,
    RegistrationSnapshotMaterializer,
};
use cfgsync_artifacts::{ArtifactFile, ArtifactSet};

struct MyMaterializer;

impl RegistrationSnapshotMaterializer for MyMaterializer {
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSnapshot,
    ) -> Result<MaterializationResult, DynCfgsyncError> {
        if registrations.len() < 2 {
            return Ok(MaterializationResult::NotReady);
        }

        let nodes = registrations.iter().map(|registration| {
            (
                registration.identifier.clone(),
                ArtifactSet::new(vec![ArtifactFile::new(
                    "/config.yaml",
                    format!("id: {}\n", registration.identifier),
                )]),
            )
        });

        Ok(MaterializationResult::ready(
            MaterializedArtifacts::from_nodes(nodes),
        ))
    }
}
```

Serving:

```rust
use cfgsync_runtime::serve;

# async fn run() -> anyhow::Result<()> {
serve(4400, MyMaterializer).await?;
# Ok(())
# }
```

Fetching and writing artifacts:

```rust
use cfgsync_runtime::{Client, OutputMap};

# async fn run(registration: cfgsync_core::NodeRegistration) -> anyhow::Result<()> {
let outputs = OutputMap::config_and_shared(
    "/node-data/node-1/config.yaml",
    "/node-data/shared",
);

Client::new("http://127.0.0.1:4400")
    .fetch_and_write(&registration, &outputs)
    .await?;
# Ok(())
# }
```

## Compatibility

The intended public API is what the crate roots reexport today.

Some older compatibility paths still exist internally to avoid breaking current in-repo consumers, but they are not the main model and should not be treated as the recommended public surface.
