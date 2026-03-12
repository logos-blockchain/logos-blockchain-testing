# cfgsync

`cfgsync` is a small library stack for node registration and config artifact delivery.

It is meant for distributed bootstrap flows where nodes register themselves with a config service, wait until artifacts are ready, fetch one payload containing the files they need, and then write those files locally before continuing startup.

The boundary is simple. `cfgsync` owns transport, registration storage, polling, and artifact serving. The application adapter owns readiness policy and artifact generation. That keeps the library reusable without forcing application-specific bootstrap logic into core crates.

## The model

There are two ways to use `cfgsync`.

The simpler path is static bundle serving. In that mode, all artifacts are known ahead of time and the server just serves a precomputed bundle.

The more general path is registration-backed serving. In that mode, nodes register first, the server builds a stable registration snapshot, and the application materializer decides when artifacts are ready and what should be served.

Both paths use the same client protocol and the same artifact payload shape. The difference is only where artifacts come from.

## Crate roles

### `cfgsync-artifacts`

This crate defines the file-level data model. `ArtifactFile` represents one file and `ArtifactSet` represents a group of files delivered together. If you only need to talk about files and file groups, this is the crate you use.

### `cfgsync-core`

This crate defines the protocol and the low-level server/client pieces. The central types are `NodeRegistration`, `RegistrationPayload`, `NodeArtifactsPayload`, `CfgsyncClient`, and the `NodeConfigSource` implementations used by the server.

It also defines the generic HTTP contract: nodes `POST /register`, then `POST /node` to fetch artifacts. The server responds with either a payload, `NotReady`, or `Missing`.

### `cfgsync-adapter`

This crate is the application-facing integration layer. The main concepts are `RegistrationSnapshot`, `RegistrationSnapshotMaterializer`, `MaterializedArtifacts`, and `MaterializationResult`.

The adapter answers one question: given the current registration snapshot, are artifacts ready yet, and if so, what should be served?

The crate also includes reusable wrappers such as `CachedSnapshotMaterializer`, `PersistingSnapshotMaterializer`, and `RegistrationConfigSource`. Static deployment-driven rendering still exists, but it lives under `cfgsync_adapter::static_deployment` as a secondary helper path. The main cfgsync model is registration-backed materialization.

### `cfgsync-runtime`

This crate provides operational helpers and binaries. It includes client-side fetch/write helpers, server config loading, and direct server entrypoints for materializers. Use this crate when you want to run cfgsync rather than define its protocol or adapter contracts.

## Artifact model

`cfgsync` serves one node request at a time, but the adapter usually thinks in snapshots.

The adapter produces `MaterializedArtifacts`, which contain node-local artifacts keyed by node identifier plus optional shared artifacts delivered alongside every node. When one node requests config, cfgsync resolves that node’s local files, merges in the shared files, and returns a single payload.

This is why applications do not need separate “node config” and “shared config” endpoints unless they want legacy compatibility.

## Registration-backed flow

This is the main integration path.

The node sends a `NodeRegistration` containing a stable identifier, an IP address, and optional typed application metadata. That metadata is opaque to cfgsync itself and is only interpreted by the application adapter.

The server stores registrations and builds a `RegistrationSnapshot`. The application implements `RegistrationSnapshotMaterializer` and decides whether the current snapshot is ready, which node-local artifacts should be produced, and which shared artifacts should accompany them.

If the materializer returns `NotReady`, cfgsync responds accordingly and the client can retry later. If it returns `Ready`, cfgsync serves the resolved artifact payload.

## Static bundle flow

Static bundle mode still exists because it is useful when artifacts are already known.

That is appropriate for fully precomputed topologies, deterministic fixtures, and test setups where no runtime coordination is needed. In that mode, cfgsync serves from `NodeArtifactsBundle` through `BundleConfigSource`.

Bundle mode is useful, but it is not the defining idea of the library anymore. The primary model is registration-backed materialization, and the static helpers are intentionally kept off the main adapter surface.

## Example: typed registration metadata

```rust
use cfgsync_core::NodeRegistration;

#[derive(serde::Serialize)]
struct MyNodeMetadata {
    network_port: u16,
    api_port: u16,
}

let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().unwrap())
    .with_metadata(&MyNodeMetadata {
        network_port: 3000,
        api_port: 18080,
    })?;
```

## Example: snapshot materializer

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

## Example: serving cfgsync

```rust
use cfgsync_runtime::serve_snapshot_cfgsync;

# async fn run() -> anyhow::Result<()> {
serve_snapshot_cfgsync(4400, MyMaterializer).await?;
# Ok(())
# }
```

## Example: fetching artifacts

```rust
use cfgsync_runtime::{ArtifactOutputMap, fetch_and_write_artifacts};

# async fn run(registration: cfgsync_core::NodeRegistration) -> anyhow::Result<()> {
let outputs = ArtifactOutputMap::new()
    .route("/config.yaml", "/node-data/node-1/config.yaml")
    .route("deployment-settings.yaml", "/node-data/shared/deployment-settings.yaml");

fetch_and_write_artifacts(&registration, "http://127.0.0.1:4400", &outputs).await?;
# Ok(())
# }
```

## What belongs in the adapter

The adapter should own the application-specific parts of bootstrap: the registration payload type, the readiness rule, the conversion from registration snapshots into artifacts, and any shared artifact generation your app needs. In practice that means things like waiting for `n` initial nodes, deriving peer lists from registrations, building node-local config files, or generating one shared deployment file for all nodes.

## What does not belong in cfgsync core

Do not push application-specific topology semantics, genesis or deployment generation, command/state-machine logic, or domain-specific ideas of what a node means into generic cfgsync. Those belong in the adapter or the consuming application.

## Recommended integration path

If you are integrating a new app, the shortest sensible path is to define a typed registration payload, implement `RegistrationSnapshotMaterializer`, return node-local and optional shared artifacts, serve them with `serve_snapshot_cfgsync(...)`, and use `CfgsyncClient` or the runtime helpers on the node side. That gives you the main library value without forcing extra application logic into cfgsync itself.

## Compatibility

The primary supported surface is what is reexported from the crate roots.

Some older names and compatibility paths still exist internally, but they are not the intended public API.
