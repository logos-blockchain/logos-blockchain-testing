# Part II — Composing Applications

The app layer deploys heterogeneous systems as one unit and exposes typed handles to workloads.

Use this entry pattern when the system under test is not a single uniform cluster. A root `AppDeployment` deploys children such as processes, uniform child clusters, and in-process services. It exposes their handles, while the scenario runtime schedules test behavior and cleanup.

- [AppHost and with_app](app-host.md) — hosting a composed app inside a scenario
- [AppDeployment and DeployContext](app-deployment.md) — the deployment contract and its context
- [Handle Ownership and Teardown](handles-teardown.md) — typed access, managed lifetime, and reverse cleanup
- [One Binary: LocalProcessApp](local-process-app.md) — the smallest building block
- [Uniform Child Clusters: LocalAppCluster](local-app-cluster.md) — a managed cluster as one component
- [Composing Heterogeneous Stacks](composing-stacks.md) — the root-app pattern, end to end
- [Backend Scope](app-backend-scope.md) — what the app layer supports today
