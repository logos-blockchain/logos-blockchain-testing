# Troubleshooting

This chapter collects common failure modes, the exact error text, and what to change.

Every error message quoted here comes from an error type in the current source. When in doubt, preserve the run and read the generated configs first; see [Diagnostics and Retained Artifacts](diagnostics.md).

---

## "duplicate runtime extension type registered: … AppRuntime"

**Symptom:** scenario preparation fails immediately with this message (raised in `core/src/scenario/runtime/extensions.rs`).

**Cause:** two `with_app(...)` calls on one scenario builder. Each `with_app` installs an `AppDeploymentFactory`, and every factory produces the same runtime extension type (`AppRuntime`); the second registration is rejected. The same error appears for any other runtime extension type registered twice.

**Fix:** a scenario has one `with_app`. To deploy several applications, compose them inside one root `AppDeployment` that deploys and exposes each child through the `DeployContext`, as the multi_app fixture's `JobStackApp` does; see [Composing Heterogeneous Stacks](composing-stacks.md).

---

## Readiness Timeout on Deploy

**Symptom:** deploy fails with `readiness probe timed out: …` (`ReadinessError::ProbeTimeout`), or `cluster stabilization timed out after …`. The processes may have spawned; they just never answered.

**Causes, in observed order of likelihood:**

1. **Wrong binary.** The binary env var points at a stale or wrong executable, so the process starts and exits (or listens on nothing). Check the interleaved process output for an immediate crash.
2. **Wrong readiness path.** The HTTP probe hits `Application::node_readiness_path()` (default `/`). If your node serves health on `/health` and you did not override the path, the probe 404s forever; see [Ports, Peers, Node Config, and Readiness](node-config.md).
3. **Port conflicts.** Local ports are preallocated by binding port 0, but another process can grab a port between reservation and spawn, or the node config may hardcode a busy port. Preserve the run and check the ports in the rendered `config.yaml`.
4. **Slow machine.** On loaded CI runners, set `SLOW_TEST_ENV=true` to double timeouts, or attach a `RetryPolicy` / relax the requirement to `HttpReadinessRequirement::AnyNodeReady` via [deployment policies](deployment-policies.md).

For a `LocalProcessApp` with `.with_readiness(...)`, a readiness failure stops the process and fails the deployment with your closure's error, and the same diagnosis applies.

---

## Binary Resolution Failures

All variants live in `BinaryProviderError` (`deployers/local/src/binary/types.rs`); see [Binary Providers](binary-providers.md).

| Message | Meaning | Fix |
|---|---|---|
| `binary could not be resolved by provider …` | `NotFound` — no provider in the chain produced a path. For a bare `EnvBinaryProvider` this means the env var is unset **or does not point at an existing file** | set the variable to a real executable path, or add a build/download fallback |
| `build command failed with status …` | `BuildFailed` — the `BuildCommand` exited non-zero | run the command by hand from the provider's `working_dir` |
| `build command did not produce configured binary output …` | `MissingBuildOutput` — build succeeded but `output_path` is missing | fix the `output_path` (profile/target dir mismatch is typical) |
| `download provider requires env var … to contain a binary URL` | `MissingDownloadUrl` — `DownloadUrl::Env` variable unset | export the URL variable |
| `failed to download binary from …` | `Download` — HTTP failure | check URL and network |
| `downloaded binary sha256 mismatch for …: expected …, got …` | `ChecksumMismatch` — bytes did not match the pinned SHA-256 | update the pinned checksum or investigate the source |
| `download processor … failed` / `… did not produce binary output …` | processor error after a verified download | debug the `DownloadProcessor` (archive layout changed?) |
| `binary path must be absolute: …` | `RelativePath` — `PathBinaryProvider` got a relative path | pass an absolute path |
| `timed out waiting for binary provider lock …` | `LockTimeout` — another process held the cross-process lock for over 10 minutes | if no other test run is alive, a crashed process left a stale lock file (under `.tf-binaries` / `target/.tf-binaries`); delete it |

---

## Docker and Compose

**`docker does not appear to be available on this host`** (`ComposeRunnerError::DockerUnavailable`): the runner probes `docker info` before deploying. Start the Docker daemon. The example binaries treat this as a graceful skip; your CI should probably not (see [Continuous Integration](ci.md)).

**`docker image '<image>' is not available; build or load it locally`** (`MissingImage`): the deployer checks every node image with `docker image inspect` and never builds or pulls. Build the app image (e.g. `docker build -f examples/kvstore/Dockerfile -t kvstore-node:local .`) or `docker pull` the upstream one, or point the `<PREFIX>_IMAGE` variable at an image you have.

**`docker compose up exited with status …` / `… timed out after …`** (`ComposeCommandError`): the stack itself failed to start. Re-run with `COMPOSE_RUNNER_PRESERVE=1` and inspect the preserved workspace and `docker compose logs` for the project.

For Kubernetes, an unreachable cluster surfaces as `K8sRunnerError::ClientInit` at deploy time; `scripts/run/checks.sh` diagnoses context, Helm, and image visibility (a `:local` tag is not visible inside `kind`/`minikube` without loading it).

---

## App Handles

**`app handle is not exposed: <type> [named "…"]`** (`AppDeployError::HandleMissing`): a workload called `require_app::<T>()` (or a deployment called `require`) for a handle that was not exposed. `ctx.deploy(app)` returns a handle without exposing it, which allows intermediate handles. Use `deploy_and_expose`, or call `ctx.expose(handle)` explicitly. Only the root deployment's own handle is auto-exposed, and only when nothing of that type was exposed already. For named lookups, the name must match the `expose_named` string exactly. See [AppDeployment and DeployContext](app-deployment.md).

**`app handle is already exposed: <type> [named "…"]`** (`AppDeployError::DuplicateHandle`): one unnamed handle per concrete type. Duplicate exposure is always an error, never a silent replacement. For two instances of the same type (two kvstore clusters), expose each under a distinct name with `expose_named`, and fetch with `require_app_named`.

---

## Teardown Surprises

App-layer resources acquired through framework adapters are owned by scenario cleanup, not by handle clone counts. If a managed process or cluster survives a test, check whether custom deployment code started it outside `LocalProcessApp` or `deploy_cluster`, or whether backend cleanup logged a failure. Managed app cleanup runs in reverse acquisition order; see [Handle Ownership and Teardown](handles-teardown.md).

Backend cleanup failures do not fail an otherwise green run: the compose and k8s deployers log them as `warn!` events with context fields (e.g. `docker compose down failed`, `helm uninstall failed during cleanup` with `release` and `namespace`). If containers or namespaces accumulate, scan your logs for those warnings and clean up manually.

When preservation is enabled, nodes or their directories remain after teardown. If they accumulate, check `TF_KEEP_LOGS`, `COMPOSE_RUNNER_PRESERVE`, and `K8S_RUNNER_PRESERVE` in your shell; `scripts/run/checks.sh` prints their current values.
