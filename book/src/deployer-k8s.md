# Kubernetes Deployer

`K8sDeployer` installs each scenario as a Helm release in a throwaway namespace and reaches nodes through NodePorts or port-forwards.

The k8s deployer lives in the `testing-framework-runner-k8s` crate. It talks to whatever cluster your current kubeconfig context points at (`kube::Client::try_default()`), installs a Helm release, waits for the workloads, and builds node clients against externally reachable ports.

```rust,ignore
use kvstore_runtime_ext::KvK8sDeployer; // = K8sDeployer<KvEnv>
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_k8s::K8sRunnerError;

let deployer = KvK8sDeployer::new();
let runner = match deployer.deploy(&scenario).await {
    Ok(runner) => runner,
    Err(K8sRunnerError::ClientInit { .. }) => return Ok(()), // no cluster available
    Err(error) => return Err(error.into()),
};
runner.run(&mut scenario).await?;
```

Run the demonstration binary with `cargo run -p kvstore-examples --bin kvstore_k8s_convergence`.

---

## Charts and Values

The environment trait `K8sDeployEnv` produces installable assets via `prepare_assets`, returning a `PreparedK8sStack`. Two asset shapes exist:

- **Generated single-template charts.** Apps implementing `K8sBinaryApp` get the standard shape for free: `render_binary_config_node_manifest` renders one ConfigMap (the serialized node config), one Deployment (single replica, `--config` arg, config mounted from the ConfigMap), and one NodePort Service per node, then `render_manifest_chart_assets` wraps them in a minimal chart (`RenderedHelmChartAssets`).
- **Real chart directories.** `NodeRuntimeSpec` builds `RunnerChartValues` (node image, pull policy, fullname override, asset mount layout, node group, optional shared bootstrap service with cfgsync configs and start scripts) and a `HelmReleaseBundle` with `--set` values and `--set-file` entries for start scripts and bootstrap configs. `RunnerAssetLayout` fixes where bootstrap configs and runner scripts land inside the chart's mount path.

Node images are resolved from env vars: for a conventional `BinaryConfigK8sSpec` the primary override is `<APP>_K8S_IMAGE`, the fallback `<APP>_IMAGE`, and the default `<binary-name>:local` with `imagePullPolicy: IfNotPresent`.

Each run installs into fresh identifiers: namespace `tf-testnet-<timestamp>-<pid>`, release `tf-runner` (override via `K8sDeployEnv::cluster_identifiers`).

---

## Lifecycle Waits

After `helm install`, the deployer waits in stages:

1. **Deployment readiness**: each node Deployment must report ready replicas (timeout `K8S_RUNNER_DEPLOYMENT_TIMEOUT_SECS`, default 180 s).
2. **Port discovery**: each node Service must have allocated NodePorts for the API and auxiliary ports declared by `collect_port_specs`.
3. **HTTP readiness**: nodes are probed over their NodePorts at `node_readiness_path()`. The probe host is `K8S_RUNNER_NODE_HOST` if set, else `KUBERNETES_SERVICE_HOST`, else `127.0.0.1`. If NodePort probing fails (common when the cluster's node IPs are not routable from the runner), the deployer transparently falls back to `kubectl port-forward` per service and probes over `127.0.0.1`.
4. **Policy-gated cluster readiness**: a final probe pass controlled by `DeploymentPolicy.readiness_enabled` / `readiness_requirement` and the deployer's `with_readiness(bool)` switch. See [Readiness, Retry, and Artifact Preservation](deployment-policies.md).

HTTP wait tuning: `K8S_RUNNER_HTTP_TIMEOUT_SECS` (default 240), `K8S_RUNNER_HTTP_PROBE_TIMEOUT_SECS` (default 30), `K8S_RUNNER_HTTP_POLL_INTERVAL_SECS` (default 1).

---

## Node Control

The Kubernetes deployer does not wire a node-control handle into managed scenario deployments. A scenario built with `with_node_control()` compiles against this backend, but runtime restart calls fail. For node lifecycle control on Kubernetes, use the Kubernetes `ManualCluster` below.

---

## Manual Mode

`K8sDeployer::manual_cluster_from_descriptors(descriptors)` (or `ManualCluster::from_topology`) installs the same Helm release, discovers every node's ports, then **scales all node Deployments to zero** so your code decides when each node starts:

```rust,ignore
let deployer = OpenRaftKvK8sDeployer::new();
let cluster = deployer
    .manual_cluster_from_descriptors(OpenRaftKvTopology::new(3))
    .await?;

cluster.start_node("node-0").await?;
cluster.start_node("node-1").await?;
cluster.wait_network_ready().await?;
cluster.restart_node("node-0").await?;
cluster.stop_all();
```

Start, stop, and restart are implemented by patching Deployment replicas between 0 and 1 and waiting for the rollout. `start_node_with` accepts `StartNodeOptions`, with two k8s-specific limits: `persist_dir` / `snapshot_dir` are rejected, and peer selection or config overrides require the environment to implement cfgsync override artifacts (`cfgsync_service` + `build_cfgsync_override_artifacts`); the override is pushed to the in-cluster cfgsync service through a temporary port-forward before the node starts. The failover demonstration uses this path end to end: `cargo run -p openraft-kv-examples --bin openraft_kv_k8s_failover`. Contrast with the declarative local variant in [ManualCluster: Imperative Node Control](manual-cluster.md).

---

## Attaching to an Existing Cluster

Existing-cluster mode is supported with a k8s descriptor: `with_existing_cluster(ExistingCluster::for_k8s_selector("app.kubernetes.io/instance=tf-runner"))` (optionally namespaced with `for_k8s_selector_in_namespace`). Services matching the selector are listed, each service's single TCP NodePort (or the port named `http`/`api`) becomes the node endpoint, and clients are built via `Application::external_node_client`. `deploy_with_metadata` returns `K8sDeploymentMetadata` (namespace + label selector) so a later scenario can attach to the stack this one installed. See [Existing and External Clusters](external-clusters.md).

---

## Observability and Cleanup

Observability inputs resolve exactly as in compose (`LOGOS_BLOCKCHAIN_*` env vars merged with the scenario capability), and `TESTNET_PRINT_ENDPOINTS` prints Prometheus/Grafana and per-node pprof endpoints. Cleanup uninstalls the Helm release and deletes the namespace (Kubernetes API first, `kubectl delete namespace` fallback), after killing any port-forward processes. Set `K8S_RUNNER_PRESERVE` to keep the release and namespace for inspection.

---

**Requirements recap:**

| Requirement | Why |
|---|---|
| Reachable cluster in current kubeconfig context | `Client::try_default()` at deploy time |
| `helm` on PATH | Release install/uninstall |
| `kubectl` on PATH | Port-forward fallback, namespace-delete fallback |
| Node images loadable by the cluster | `<APP>_K8S_IMAGE` / `<APP>_IMAGE` / `<binary>:local` |
