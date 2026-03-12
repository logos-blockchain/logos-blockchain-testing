# cfgsync

`cfgsync` is a small library stack for node registration and config artifact delivery.

It is designed for distributed test and bootstrap flows where nodes need to:
- register themselves with a config service
- wait until config is ready
- fetch one artifact payload containing the files they need
- write those files locally and continue startup

The important boundary is:
- `cfgsync` owns transport, registration storage, polling, and artifact serving
- the application adapter owns readiness policy and artifact generation

That keeps `cfgsync` generic while still supporting app-specific bootstrap logic.

## Crates

### `cfgsync-artifacts`
Data types for delivered files.

Primary types:
- `ArtifactFile`
- `ArtifactSet`

Use this crate when you only need to talk about files and file collections.

### `cfgsync-core`
Protocol and server/client building blocks.

Primary types:
- `NodeRegistration`
- `RegistrationPayload`
- `NodeArtifactsPayload`
- `CfgsyncClient`
- `NodeConfigSource`
- `StaticConfigSource`
- `BundleConfigSource`
- `CfgsyncServerState`
- `build_cfgsync_router(...)`
- `serve_cfgsync(...)`

This crate defines the generic HTTP contract:
- `POST /register`
- `POST /node`

Typical flow:
1. client registers a node
2. client requests its artifacts
3. server returns either:
   - `Ready` payload
   - `NotReady`
   - `Missing`

### `cfgsync-adapter`
Adapter-facing materialization layer.

Primary types:
- `MaterializedArtifacts`
- `RegistrationSnapshot`
- `RegistrationSnapshotMaterializer`
- `CachedSnapshotMaterializer`
- `RegistrationConfigSource`
- `DeploymentAdapter`

This crate is where app-specific bootstrap logic plugs in.

The main pattern is snapshot materialization:
- `RegistrationSnapshotMaterializer`

Use it when readiness depends on the full registered set.

### `cfgsync-runtime`
Small runtime helpers and binaries.

Primary exports:
- `ArtifactOutputMap`
- `register_and_fetch_artifacts(...)`
- `fetch_and_write_artifacts(...)`
- `run_cfgsync_client_from_env()`
- `CfgsyncServerConfig`
- `CfgsyncServingMode`
- `serve_cfgsync_from_config(...)`
- `serve_snapshot_cfgsync(...)`

This crate is for operational wiring, not for app-specific logic.

## Design

There are two serving models.

### 1. Static bundle serving
Config is precomputed up front.

Use:
- `NodeArtifactsBundle`
- `BundleConfigSource`
- `CfgsyncServingMode::Bundle`

This is the simplest path when the full artifact set is already known.

### 2. Registration-backed serving
Config is produced from node registrations.

Use:
- `RegistrationSnapshotMaterializer`
- `CachedSnapshotMaterializer`
- `RegistrationConfigSource`
- `serve_snapshot_cfgsync(...)`

This is the right model when config readiness depends on the current registered set.

## Public API shape

### Register a node

Nodes register with:
- stable identifier
- IPv4 address
- optional typed application metadata

Application metadata is carried as an opaque serialized payload:
- generic in `cfgsync`
- interpreted only by the adapter

Example:

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

### Materialize from the registration snapshot

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

        let nodes = registrations
            .iter()
            .map(|registration| (
                registration.identifier.clone(),
                ArtifactSet::new(vec![ArtifactFile::new(
                    "/config.yaml",
                    format!("id: {}\n", registration.identifier),
                )]),
            ));

        Ok(MaterializationResult::ready(
            MaterializedArtifacts::from_nodes(nodes),
        ))
    }
}
```

### Serve registration-backed cfgsync

```rust
use cfgsync_runtime::serve_snapshot_cfgsync;

# async fn run() -> anyhow::Result<()> {
serve_snapshot_cfgsync(4400, MyMaterializer).await?;
# Ok(())
# }
```

### Fetch from a client

```rust
use cfgsync_core::CfgsyncClient;

# async fn run(registration: cfgsync_core::NodeRegistration) -> anyhow::Result<()> {
let client = CfgsyncClient::new("http://127.0.0.1:4400");
client.register_node(&registration).await?;
let payload = client.fetch_node_config(&registration).await?;
# Ok(())
# }
```

### Fetch and write artifacts with runtime helpers

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

Put these in your app adapter:
- registration payload type
- readiness rule
- conversion from registration snapshot to artifacts
- shared file generation if your app needs shared files

Examples:
- wait for `n` initial nodes
- derive peer lists from registrations
- build node-local config files
- include shared deployment/config files in every node payload

## What does **not** belong in `cfgsync-core`

Do not put these into generic cfgsync:
- app-specific topology rules
- domain-specific genesis/deployment generation
- app-specific command/state-machine logic
- service-specific semantics for what a node means

Those belong in the adapter or the consuming application.

## Recommended integration model

If you are integrating a new app, start here:

1. define a typed registration payload
2. implement `RegistrationSnapshotMaterializer`
3. return one artifact payload per node
4. include shared files inside that payload if your app needs them
5. serve with `serve_snapshot_cfgsync(...)`
6. use `CfgsyncClient` on the node side
7. use runtime helpers if you want generic client-side file writing instead of custom dispatch code

This model keeps the generic library small and keeps application semantics where they belong.

## Compatibility

The primary surface is the one reexported from crate roots.

There are hidden compatibility aliases in some crates to keep older internal consumers building, but they are not the recommended API for new integrations.

## Runtime config files

`serve_cfgsync_from_config(...)` is for runtime-config-driven serving.

Today it supports:
- static bundle serving
- registration serving from a prebuilt artifact catalog

If your app has a real registration-backed materializer, prefer the direct runtime API:
- `serve_snapshot_cfgsync(...)`

That keeps application behavior in adapter code instead of trying to encode it into YAML.

## Current status

`cfgsync` is suitable for:
- internal reuse across multiple apps
- registration-backed bootstrap flows
- static precomputed artifact serving

It is not intended to be:
- a generic orchestration framework
- a topology engine
- a secret-management system
- an app-specific bootstrap policy layer
