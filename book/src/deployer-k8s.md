# Kubernetes Backend

`K8sClusterProvisioner` installs each cluster as a Helm release in a throwaway namespace and reaches nodes through NodePorts or port-forwards.

The Kubernetes backend lives in the `testing-framework-runner-k8s` crate. It talks to whatever cluster your current kubeconfig context points at (`kube::Client::try_default()`), installs a Helm release, waits for the workloads, and builds node clients against externally reachable ports. Managed provisioning is built on the same `ManualCluster` unit that imperative tests use — one lifecycle implementation for both.

```rust,ignore
use testing_framework_runner_k8s::{K8sClusterProvisioner, ManualClusterError};

let mut scenario = AppHost::scenario()
    .with_app_using(
        ClusterApp::<KvEnv>::new(KvTopology::new(3)),
        K8sClusterProvisioner,
    )
    .build()?;

let runner = match AppHostDeployer.deploy(&scenario).await {
    Ok(runner) => runner,
    Err(error) if k8s_unavailable(&error) => return Ok(()), // no cluster available
    Err(error) => return Err(error.into()),
};
runner.run(&mut scenario).await?;
```

(`k8s_unavailable` downcasts the deploy error's source to `ManualClusterError::ClientInit`, or an `InstallStack` whose message indicates an unreachable API server; the shipped k8s bins show the exact helper.) Run the demonstration binary with `cargo run -p kvstore-examples --bin kvstore_k8s_convergence`.

---

## Charts and Values

The environment trait `K8sDeployEnv` produces installable assets via `prepare_assets`, returning a `PreparedK8sStack`. Two asset shapes exist:

- **Generated single-template charts.** Apps implementing `K8sBinaryApp` get the standard shape for free: `render_binary_config_node_manifest` renders one ConfigMap (the serialized node config), one Deployment (single replica, `--config` arg, config mounted from the ConfigMap), and one NodePort Service per node, then `render_manifest_chart_assets` wraps them in a minimal chart (`RenderedHelmChartAssets`).
- **Real chart directories.** `NodeRuntimeSpec` builds `RunnerChartValues` (node image, pull policy, fullname override, asset mount layout, node group, optional shared bootstrap service with cfgsync configs and start scripts) and a `HelmReleaseBundle` with `--set` values and `--set-file` entries for start scripts and bootstrap configs. `RunnerAssetLayout` fixes where bootstrap configs and runner scripts land inside the chart's mount path.

Node images are resolved from env vars: for a conventional `BinaryConfigK8sSpec` the primary override is `<APP>_K8S_IMAGE`, the fallback `<APP>_IMAGE`, and the default `<binary-name>:local` with `imagePullPolicy: IfNotPresent`.

Each run installs into fresh identifiers: namespace `tf-testnet-<timestamp>-<pid>`, release `tf-runner` (override via `K8sDeployEnv::cluster_identifiers`).

---

## Lifecycle Waits

After `helm install`, an eagerly started cluster waits in stages:

1. **Deployment readiness**: each node Deployment must report ready replicas (timeout `K8S_RUNNER_DEPLOYMENT_TIMEOUT_SECS`, default 180 s).
2. **Port discovery**: each node Service must have allocated NodePorts for the API and auxiliary ports declared by `collect_port_specs`.
3. **HTTP readiness**: nodes are probed over their NodePorts at `node_readiness_path()`. The probe host is `K8S_RUNNER_NODE_HOST` if set, else `KUBERNETES_SERVICE_HOST`, else `127.0.0.1`. If NodePort probing fails (common when the cluster's node IPs are not routable from the runner), the backend transparently falls back to `kubectl port-forward` per service and probes over `127.0.0.1`.
4. **Policy-gated cluster readiness**: a final probe pass controlled by the request's `DeploymentPolicy.readiness_enabled` / `readiness_requirement`. See [Readiness, Retry, and Artifact Preservation](deployment-policies.md).

HTTP wait tuning: `K8S_RUNNER_HTTP_TIMEOUT_SECS` (default 240), `K8S_RUNNER_HTTP_PROBE_TIMEOUT_SECS` (default 30), `K8S_RUNNER_HTTP_POLL_INTERVAL_SECS` (default 1).

---

## Node Control and Start Modes

When the request asks for `ClusterControlRequest::Full` — the `ClusterApp` default — the handle supports `start_node`, `stop_node`, `restart_node`, and `wait_node_ready`, implemented by patching each per-node Deployment's replicas between 0 and 1 and waiting for the rollout. It works over both direct NodePorts and the `kubectl port-forward` fallback — after a restart or start the node's forwards are respawned on their original local ports, so existing clients keep working. Two k8s-specific limits on `start_node_with(StartNodeOptions)`: `persist_dir` / `snapshot_dir` are rejected, and peer selection or config overrides require the environment to implement cfgsync override artifacts (`cfgsync_service` + `build_cfgsync_override_artifacts`); the override is pushed to the in-cluster cfgsync service through a temporary port-forward before the node starts.

`ClusterStartMode` maps directly onto the lifecycle: `Eager` installs and starts every node before the handle is returned (`FrameworkManaged`); `OnDemand` installs the release with **all node Deployments scaled to zero** so your code decides when each node starts (`ManualControlled`). `cargo run -p kvstore-examples --bin kvstore_k8s_restart_chaos` exercises managed control end to end with the restart chaos workload ([Chaos and Controlled Failure](chaos.md)).

Attached clusters get the same restart control: node names resolve to their Deployments in the attached namespace at call time, so a cluster whose workloads do not follow the one-Deployment-per-node convention fails per call with an error naming the deployment and namespace. Only default restart options are accepted (attached clusters have no cfgsync), and port-forwarded access respawns the affected forward after each restart.

---

## Imperative Use: ManualCluster

The same unit is available directly for code that owns the sequence:

```rust,ignore
use testing_framework_runner_k8s::ManualCluster;

let cluster = ManualCluster::<OpenRaftKvEnv>::from_topology(OpenRaftKvTopology::new(3)).await?;

cluster.start_node("node-0").await?;
cluster.start_node("node-1").await?;
cluster.wait_network_ready().await?;
cluster.restart_node("node-0").await?;
cluster.stop_all();
```

`from_topology` is the on-demand default; `ManualCluster::provision(topology, start_mode, policy, &observability)` selects eager start, a non-default `DeploymentPolicy`, or explicit observability inputs. The failover demonstration uses this path end to end: `cargo run -p openraft-kv-examples --bin openraft_kv_k8s_failover`. Contrast with the local variant in [ManualCluster: Imperative Node Control](manual-cluster.md).

---

## Attaching to an Existing Cluster

Attached requests are supported with a k8s descriptor: `ClusterRequest::attached(ExistingCluster::for_k8s_selector("app.kubernetes.io/instance=tf-runner"))` (optionally namespaced with `for_k8s_selector_in_namespace`). Services matching the selector are listed, each service's single TCP NodePort (or the port named `http`/`api`) becomes the node endpoint, and clients are built via `Application::external_node_client`. When endpoints are not reachable from the runner, discovery falls back to `kubectl port-forward` per service. The handle carries a readiness wait, and requesting `Full` control wires deployment-scaling restart just like managed clusters. See [Existing and External Clusters](external-clusters.md).

---

## Observability and Cleanup

Observability inputs resolve exactly as in compose (`LOGOS_BLOCKCHAIN_*` env vars merged with the request's per-cluster values), and `TESTNET_PRINT_ENDPOINTS` prints Prometheus/Grafana and per-node pprof endpoints. Cleanup uninstalls the Helm release and deletes the namespace (Kubernetes API first, `kubectl delete namespace` fallback), after killing any port-forward processes. Set `K8S_RUNNER_PRESERVE` to keep the release and namespace for inspection.

There is no k8s `ContainerStackProvisioner` yet: heterogeneous container stacks run on the compose backend ([Backend Scope](app-backend-scope.md)).

---

**Requirements recap:**

| Requirement | Why |
|---|---|
| Reachable cluster in current kubeconfig context | `Client::try_default()` at deploy time |
| `helm` on PATH | Release install/uninstall |
| `kubectl` on PATH | Port-forward fallback, namespace-delete fallback |
| Node images loadable by the cluster | `<APP>_K8S_IMAGE` / `<APP>_IMAGE` / `<binary>:local` |
