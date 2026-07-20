# Compose Deployer

`ComposeDeployer` runs each node as a Docker Compose service generated from your deployment descriptor.

The compose deployer lives in the `testing-framework-runner-compose` crate. It generates a compose file per run, brings the stack up, discovers the host ports Docker assigned, probes readiness, and hands control to the scenario runner. It requires a running Docker daemon; otherwise deployment returns `ComposeRunnerError::DockerUnavailable`.

```rust,ignore
use kvstore_runtime_ext::KvComposeDeployer; // = ComposeDeployer<KvEnv>
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;

let deployer = KvComposeDeployer::new();
let runner = match deployer.deploy(&scenario).await {
    Ok(runner) => runner,
    Err(ComposeRunnerError::DockerUnavailable) => return Ok(()), // skip without Docker
    Err(error) => return Err(error.into()),
};
runner.run(&mut scenario).await?;
```

Run the demonstration binary with `cargo run -p kvstore-examples --bin kvstore_compose_convergence`.

---

## Deployment Pipeline

```mermaid
flowchart LR
    A[Workspace<br/>tempdir] --> B[Write configs<br/>+ cfgsync.yaml]
    B --> C[Render<br/>compose.generated.yml]
    C --> D[docker compose<br/>create + up]
    D --> E[Port discovery<br/>docker compose port]
    E --> F[Readiness<br/>probes]
    F --> G[Node clients<br/>+ Runner]
```

1. **Workspace.** A temporary `ComposeWorkspace` is created; the app's `ComposeDeployEnv::prepare_compose_configs` writes per-node config files (for `ComposeBinaryApp` environments, one static config per node under `stack/configs/`, rewritten for service hostnames `node-0`, `node-1`, ...).
2. **cfgsync.** If the environment enables `ComposeConfigServerMode::Docker`, a cfgsync config server container is started on an ephemeral port and the deployer waits for it to accept TCP connections before proceeding. The default mode is `Disabled`. See [Static Artifacts and cfgsync](cfgsync.md).
3. **Compose file.** The env's `compose_descriptor` (image, entrypoint, volumes, ports, environment, optional platform per service) is rendered through the Tera template at `testing-framework/deployers/compose/assets/docker-compose.yml.tera` into `compose.generated.yml`. The template is resolved relative to the repository root (`CARGO_WORKSPACE_DIR` override respected). Required images are checked with `docker image inspect` up front. The deployer never builds or pulls them; a missing image fails the deploy with `MissingImage`.
4. **Bring-up.** `docker compose create` and `docker compose up` run under a unique project name (`compose-stack-<uuid>`). On failure, container logs are dumped before cleanup.
5. **Ports.** Container ports map to ephemeral host ports; the deployer resolves each with `docker compose port` and records them as `NodeHostPorts { api, testing }`. The host defaults to `127.0.0.1` and can be overridden with `COMPOSE_RUNNER_HOST`.
6. **Readiness.** Per the env's `ComposeReadinessProbe`: HTTP GET against `Application::node_readiness_path()` on each mapped API port, or raw TCP reachability. Gated by `DeploymentPolicy.readiness_enabled` and the deployer's own `with_readiness(bool)` switch; when disabled, the stack gets a short fixed grace period instead. See [Readiness, Retry, and Artifact Preservation](deployment-policies.md).
7. **Clients.** `build_node_client` runs against the discovered host/port pairs, producing the scenario's typed node clients.

---

## Node Control

With `with_node_control()` on the builder, the deployer installs a `ComposeNodeControl` handle bound to the generated compose file and project. It supports **restart only**: `restart_node(name)` shells out to `docker compose restart <service>`. Start and stop of individual services are not wired for managed compose scenarios. The openraft_kv failover scenario runs on this backend: `cargo run -p openraft-kv-examples --bin openraft_kv_compose_failover`.

---

## Attaching to an Existing Stack

The compose deployer fully supports existing-cluster mode. A scenario built with `with_existing_cluster(ExistingCluster::for_compose_project("my-project"))` skips workspace generation entirely: services are discovered from the running project (or taken from `for_compose_services`), each container's labeled API port is inspected, and clients are built through `Application::external_node_client`. In this mode node control gains `stop_node` in addition to `restart_node`, implemented with `docker container stop` / `docker container restart` against discovered container IDs.

`deploy_with_metadata` returns `ComposeDeploymentMetadata` alongside the runner; its `existing_cluster()` / `IntoExistingCluster` impl lets a later scenario attach to the stack this one deployed. See [Existing and External Clusters](external-clusters.md).

---

## Observability

Compose resolves `ObservabilityInputs` by merging `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL`, `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL`, and `LOGOS_BLOCKCHAIN_GRAFANA_URL` env vars with the scenario's observability capability (capability values win). The OTLP ingest URL is passed into config preparation so node configs can point at your collector; the metrics query URL becomes the run's Prometheus-backed `Metrics` handle. Setting `TESTNET_PRINT_ENDPOINTS` prints Prometheus/Grafana endpoints and per-node pprof profile URLs to stdout. See [Telemetry and External Observability](telemetry.md).

---

## Cleanup

The runner's cleanup guard runs `docker compose down`, shuts down the cfgsync container if one was started, and removes the workspace. Setting `COMPOSE_RUNNER_PRESERVE` (or `TESTNET_RUNNER_PRESERVE`) keeps the stack running and persists the workspace directory for post-mortem inspection; the preserved path is logged.

---

**Requirements recap:**

| Requirement | Why |
|---|---|
| Docker daemon running | `ensure_docker_available` gates every deploy |
| Node container images | Must exist locally before deploy; missing images fail with `MissingImage` |
| Repository checkout | The compose Tera template is read from the repo tree |
