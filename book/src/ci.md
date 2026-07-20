# Continuous Integration

This chapter covers how this repository checks itself, and patterns for running framework-based tests in your own CI.

---

## Workflows in This Repository

Two GitHub Actions workflows live in `.github/workflows/`.

### `lint.yml`

Runs on every push and pull request, with per-ref concurrency cancellation. All jobs pin the `nightly-2025-09-14` toolchain and cache `~/.cargo/registry`, `~/.cargo/git`, and `target/` keyed on `Cargo.lock`.

| Job | Command | Checks |
|---|---|---|
| `fmt` | `cargo +nightly-2025-09-14 fmt --all -- --check` | formatting |
| `clippy` | `cargo clippy --all --all-targets --all-features -- -D warnings` | lints, warnings as errors |
| `deny` | `cargo deny check -c .cargo-deny.toml --show-stats -D warnings` | licenses, advisories, bans |
| `taplo` | `taplo fmt --check` and `taplo lint` | TOML formatting and lints |
| `machete` | `cargo machete` | unused dependencies |

### `deploy-pages.yml`

Builds this book with `mdbook build book` and publishes `target/book` to GitHub Pages. It triggers on pushes to `master` that touch `book/**`, or manually via `workflow_dispatch`.

**Note:** CI currently covers linting and the book. The unit tests and example scenarios run via `cargo test` / `cargo run` on developer machines; there is no test workflow yet.

---

## Helper Scripts

- `scripts/run/checks.sh` is an informational, best-effort environment sanity check. It reports workspace and disk state, the Rust toolchain, Docker and Docker Compose availability, the Kubernetes context (including whether a `:local` image tag will be visible to `kind`, `minikube`, or `docker-desktop` clusters), and the current values of runner debug flags such as `COMPOSE_RUNNER_PRESERVE` and `K8S_RUNNER_PRESERVE`. Run it first when a backend misbehaves.
- `scripts/run/check-boundaries.sh` is a boundary guard for an adopter checkout living next to this repository. It fails if the adopter's topology crate references extension-specific symbols (`cfgsync`, compose/k8s deployer types), keeping the [framework/application boundary](tf-boundaries.md) enforceable by grep.

---

## Patterns for Consumers

If your repository builds tests on this framework, the following translate directly into CI configuration.

### Cache the build for `BuildBinaryProvider`

Local scenarios that resolve node binaries through a `BuildBinaryProvider` invoke `cargo build` at deploy time. On a cold runner this can dominate the job. Cache `~/.cargo/registry`, `~/.cargo/git`, and `target/` keyed on `Cargo.lock`, as `lint.yml` does, so the deploy-time build is incremental. Resolution is also cached in-process and serialized across concurrent test processes with a file lock, so parallel test binaries do not race the same build; see [Binary Providers](binary-providers.md).

### Ensure Docker for compose tests

Compose scenarios need a working Docker daemon and the node images already present: the runner verifies images with `docker image inspect` and fails with `MissingImage` rather than building or pulling. Add an image build/pull step before the test step. Decide your skip policy explicitly: the in-repo example binaries treat `ComposeRunnerError::DockerUnavailable` as a graceful skip, which is convenient locally but silently masks coverage loss in CI. In a pipeline, prefer failing (or gating the job on a Docker-capable runner).

### Slow runners

Set `SLOW_TEST_ENV=true` on constrained runners; the framework doubles its internal timeouts (`testing_framework_core::adjust_timeout`). The k8s deployer's wait timeouts can also be tuned individually; see [Environment Variables](environment-variables.md).

### Preserve artifacts on failure

By default every backend tears down and deletes its working state. To retain evidence from failed CI runs, preserve and upload the artifacts:

```yaml
- name: Run scenarios
  run: cargo test -p my-scenarios
  env:
    TF_KEEP_LOGS: "1"            # keep local per-node working directories
    COMPOSE_RUNNER_PRESERVE: "1" # keep the compose workspace and containers
- name: Upload artifacts
  if: failure()
  uses: actions/upload-artifact@v4
  with:
    name: scenario-artifacts
    path: |
      **/.tmp*
```

Local node directories are created under the test process's working directory; note that panicking tests preserve their node directories automatically. The equivalent in code is `CleanupPolicy { preserve_artifacts: true }` via `with_deployment_policy`. What lands in those directories and how to read them is covered in [Diagnostics and Retained Artifacts](diagnostics.md).

### Reproduce failures

Log or fix the deployment seed (`with_deployment_seed`) so a failing CI run can be replayed locally with the same generated deployment; see [Seeds and Reproducibility](seeds.md).
