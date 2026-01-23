# CI Integration

Both **LocalDeployer** and **ComposeDeployer** work well in CI environments. Choose based on your tradeoffs.

## Runner Comparison for CI

**LocalDeployer (Host Runner):**
- Faster startup (no Docker overhead)
- Good for quick smoke tests
- **Trade-off:** Less isolation (processes share host resources)

**ComposeDeployer (Recommended for CI):**
- Better isolation (containerized)
- Reproducible environment
- Can integrate with external Prometheus/Grafana (optional)
- **Trade-offs:** Slower startup (Docker image build), requires Docker daemon

**K8sDeployer:**
- Production-like environment
- Full resource isolation
- **Trade-offs:** Slowest (cluster setup + image loading), requires cluster access
- Best for nightly/weekly runs or production validation

**Existing Examples:**

See `.github/workflows/lint.yml` (jobs: `host_smoke`, `compose_smoke`) for CI examples running the demo scenarios in this repository.

## Complete CI Workflow Example

Here's a comprehensive GitHub Actions workflow demonstrating host and compose runners with caching, matrix testing, and log collection:

```yaml
name: Testing Framework CI

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

env:
  POL_PROOF_DEV_MODE: true
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  # Quick smoke test with host runner (no Docker)
  host_smoke:
    name: Host Runner Smoke Test
    runs-on: ubuntu-latest
    timeout-minutes: 15
    
    steps:
      - name: Checkout repository
        uses: actions/checkout@v4
      
      - name: Set up Rust toolchain
        uses: actions-rs/toolchain@v1
        with:
          profile: minimal
          toolchain: nightly
          override: true
      
      - name: Cache Rust dependencies
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: ${{ runner.os }}-cargo-host-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-host-
      
      - name: Cache nomos-node build
        uses: actions/cache@v3
        with:
          path: |
            ../nomos-node/target/release/nomos-node
          key: ${{ runner.os }}-nomos-${{ hashFiles('../nomos-node/**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-nomos-
      
      - name: Run host smoke test
        run: |
          # Use run-examples.sh which handles setup automatically
          scripts/run/run-examples.sh -t 120 -v 3 -e 1 host
      
      - name: Upload logs on failure
        if: failure()
        uses: actions/upload-artifact@v3
        with:
          name: host-runner-logs
          path: |
            .tmp/
            *.log
          retention-days: 7

  # Compose runner matrix (with Docker)
  compose_matrix:
    name: Compose Runner (${{ matrix.topology }})
    runs-on: ubuntu-latest
    timeout-minutes: 25
    
    strategy:
      fail-fast: false
      matrix:
        topology:
          - "3v1e"
          - "5v1e"
    
    steps:
      - name: Checkout repository
        uses: actions/checkout@v4
      
      - name: Set up Rust toolchain
        uses: actions-rs/toolchain@v1
        with:
          profile: minimal
          toolchain: nightly
          override: true
      
      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v2
      
      - name: Cache Rust dependencies
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: ${{ runner.os }}-cargo-compose-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-compose-
      
      - name: Cache Docker layers
        uses: actions/cache@v3
        with:
          path: /tmp/.buildx-cache
          key: ${{ runner.os }}-buildx-${{ hashFiles('Dockerfile', 'scripts/build/build_test_image.sh') }}
          restore-keys: |
            ${{ runner.os }}-buildx-
      
      - name: Run compose test
        env:
          TOPOLOGY: ${{ matrix.topology }}
        run: |
          # Build and run with the specified topology
          scripts/run/run-examples.sh -t 120 -v ${TOPOLOGY:0:1} -e ${TOPOLOGY:2:1} compose
      
      - name: Collect Docker logs on failure
        if: failure()
        run: |
          mkdir -p logs
          for container in $(docker ps -a --filter "name=nomos-compose-" -q); do
            docker logs $container > logs/$(docker inspect --format='{{.Name}}' $container).log 2>&1
          done
      
      - name: Upload logs and artifacts
        if: failure()
        uses: actions/upload-artifact@v3
        with:
          name: compose-${{ matrix.topology }}-logs
          path: |
            logs/
            .tmp/
          retention-days: 7
      
      - name: Clean up Docker resources
        if: always()
        run: |
          docker compose down -v 2>/dev/null || true
          docker ps -a --filter "name=nomos-compose-" -q | xargs -r docker rm -f

  # Summary job (requires all tests to pass)
  ci_success:
    name: CI Success
    needs: [host_smoke, compose_matrix]
    runs-on: ubuntu-latest
    if: always()
    
    steps:
      - name: Check all jobs
        run: |
          if [[ "${{ needs.host_smoke.result }}" != "success" ]] || \
             [[ "${{ needs.compose_matrix.result }}" != "success" ]]; then
            echo "One or more CI jobs failed"
            exit 1
          fi
          echo "All CI jobs passed!"
```

## Workflow Features

1. **Matrix Testing:** Runs compose tests with different topologies (`3v1e`, `5v1e`)
2. **Caching:** Caches Rust dependencies, Docker layers, and nomos-node builds for faster runs
3. **Log Collection:** Automatically uploads logs and artifacts when tests fail
4. **Timeout Protection:** Reasonable timeouts prevent jobs from hanging indefinitely
6. **Clean Teardown:** Ensures Docker resources are cleaned up even on failure

## Customization Points

**Topology Matrix:**

Add more topologies for comprehensive testing:

```yaml
matrix:
  topology:
    - "3v1e"
    - "5v1e"
    - "10v2e"  # Larger scale
```

**Timeout Adjustments:**

Increase `timeout-minutes` for longer-running scenarios or slower environments:

```yaml
timeout-minutes: 30  # Instead of 15
```

**Artifact Retention:**

Change `retention-days` based on your storage needs:

```yaml
retention-days: 14  # Keep logs for 2 weeks
```

**Conditional Execution:**

Run expensive tests only on merge to main:

```yaml
if: github.event_name == 'push' && github.ref == 'refs/heads/main'
```

## Best Practices

### Required: Set POL_PROOF_DEV_MODE

**Always set `POL_PROOF_DEV_MODE=true` globally** in your workflow env:

```yaml
env:
  POL_PROOF_DEV_MODE: true  # REQUIRED!
```

Without this, tests will hang due to expensive proof generation.

### Use Helper Scripts

Prefer `scripts/run/run-examples.sh` which handles all setup automatically:

```bash
scripts/run/run-examples.sh -t 120 -v 3 -e 1 host
```

This is more reliable than manual `cargo run` commands.

### Cache Aggressively

Cache Rust dependencies, nomos-node builds, and Docker layers to speed up CI:

```yaml
- name: Cache Rust dependencies
  uses: actions/cache@v3
  with:
    path: |
      ~/.cargo/bin/
      ~/.cargo/registry/index/
      ~/.cargo/registry/cache/
      ~/.cargo/git/db/
      target/
    key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
```

### Collect Logs on Failure

Always upload logs when tests fail for easier debugging:

```yaml
- name: Upload logs on failure
  if: failure()
  uses: actions/upload-artifact@v3
  with:
    name: test-logs
    path: |
      .tmp/
      *.log
    retention-days: 7
```

### Split Workflows for Faster Iteration

For large projects, split host/compose/k8s into separate workflow files:

- `.github/workflows/test-host.yml` — Fast smoke tests
- `.github/workflows/test-compose.yml` — Reproducible integration tests
- `.github/workflows/test-k8s.yml` — Production-like validation (nightly)

### Run K8s Tests Less Frequently

K8s tests are slower. Consider running them only on main branch or scheduled:

```yaml
on:
  push:
    branches: [main]
  schedule:
    - cron: '0 2 * * *'  # Daily at 2 AM
```

## Platform-Specific Notes

### Ubuntu Runners

- Docker pre-installed and running
- Best for compose/k8s runners
- Most common choice

### macOS Runners

- Docker Desktop not installed by default
- Slower and more expensive
- Use only if testing macOS-specific issues

### Self-Hosted Runners

- Cache Docker images locally for faster builds
- Set resource limits (`SLOW_TEST_ENV=true` if needed)
- Ensure cleanup scripts run (`docker system prune`)

## Debugging CI Failures

### Enable Debug Logging

Add debug environment variables temporarily:

```yaml
env:
  RUST_LOG: debug
  NOMOS_LOG_LEVEL: debug
```

### Preserve Containers (Compose)

Set `COMPOSE_RUNNER_PRESERVE=1` to keep containers running for inspection:

```yaml
- name: Run compose test (preserve on failure)
  env:
    COMPOSE_RUNNER_PRESERVE: 1
  run: scripts/run/run-examples.sh -t 120 -v 3 -e 1 compose
```

### Access Artifacts

Download uploaded artifacts from the GitHub Actions UI to inspect logs locally.

## Next Steps

- [Running Examples](running-examples.md) — Manual execution for local development
- [Environment Variables](environment-variables.md) — Full variable reference
- [Troubleshooting](troubleshooting.md) — Common CI-specific issues

