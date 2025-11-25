# Nomos Testing (split workspace)

This workspace contains only the testing framework crates pulled from the `nomos-node` repo:

- `testing-framework/configs`
- `testing-framework/core`
- `testing-framework/workflows`
- `testing-framework/runners` (compose, k8s, local)
- `tests/workflows` (demo/integration tests)
- helper scripts (`scripts/setup-nomos-circuits.sh`, `scripts/build-rapidsnark.sh`)

## Layout

The workspace expects a sibling checkout of `nomos-node`:

```
IdeaProjects/
├─ nomos-node/        # existing monorepo with all node crates
└─ nomos-testing/     # this workspace (you are here)
```

Path dependencies in `Cargo.toml` point to `../nomos-node/...`.

## Usage

```bash
cd nomos-testing
cargo test -p tests-workflows -- --ignored   # or any crate you need
```

If you need circuits/prover assets, run the usual helpers from this workspace:

```bash
scripts/setup-nomos-circuits.sh
scripts/build-rapidsnark.sh
```

All code is sourced from the local branches:
`feat/testing-framework-move`, `feat/testing-framework`, `feat/testing-framework-runners`, `feat/testing-framework-k8s-runner`.
