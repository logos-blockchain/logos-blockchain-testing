# Examples

Use the app layer first when the test is about an application or composed
system interface.

Canonical app-layer examples:

- `kvstore_app_host_convergence`: one local app cluster exposed through
  `AppHost`
- `openraft_kv_app_host_smoke`: one richer local app cluster exposed through
  `AppHost`
- `multi-app-e2e`: composed queue, worker, and result-store stack exercised
  through integration tests

The older direct `ScenarioBuilder<AppEnv>` examples are still useful for
backend-specific coverage:

- local uniform cluster behavior
- compose runner behavior
- k8s runner behavior
- manual cluster control

Do not use those older examples as the pattern for new composed systems. New
multi-app tests should define an `AppDeployment`, expose typed handles, and run
through `AppHost::scenario().with_app(...)`.
