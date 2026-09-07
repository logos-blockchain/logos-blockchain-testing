# Part V — Deployers and Sources

This part covers where scenarios run and where their nodes come from.

Clusters provision to local processes, Docker Compose, or Kubernetes through one request and handle model — the provisioner passed per app selects the backend — and scenarios can also attach to clusters you already operate.

- [Capability Matrix](capability-matrix.md) — feature support per deployer
- [Local Deployer](deployer-local.md) — processes on your machine
- [Compose Deployer](deployer-compose.md) — containerized clusters
- [Kubernetes Deployer](deployer-k8s.md) — Helm releases in isolated namespaces
- [Shared Cluster Provisioning](cluster-provisioning.md) — one request and handle model across cluster sources
- [Existing and External Clusters](external-clusters.md) — attaching to live systems
- [Binary Providers](binary-providers.md) — resolving node binaries: path, env, build, download
- [Readiness, Retry, and Artifact Preservation](deployment-policies.md) — deployment policy
