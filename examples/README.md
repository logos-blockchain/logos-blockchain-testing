# Examples

Every example runs through the one app-based model:
`AppHost::scenario().with_app(...)` (local processes by default) or
`.with_app_using(...)` with a Compose or Kubernetes provisioner.

Canonical entry points:

- `kvstore_basic_convergence`: one local kvstore cluster deployed as a
  `ClusterApp`
- `openraft_kv_basic_failover`: a richer consensus cluster with leader
  failover through the `ClusterHandle`
- `multi-app-e2e`: composed queue, worker, and result-store stack exercised
  through integration tests

Backend-specific coverage lives in the per-example `compose_*` and `k8s_*`
binaries, which run the same deployments through
`ComposeProvisioner::default()` or `K8sClusterProvisioner`, plus the manual
cluster examples for imperative node control.

New multi-app tests should define an `AppDeployment`, expose typed handles,
and run through `AppHost::scenario().with_app(...)`.
