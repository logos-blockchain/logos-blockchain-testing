#!/usr/bin/env bash
set -euo pipefail

RUSTFLAGS='--cfg feature="pol-dev-mode"' \
  cargo build --manifest-path /workspace/testing-framework/tools/cfgsync-runtime/Cargo.toml --bin cfgsync-server

RUSTFLAGS='--cfg feature="pol-dev-mode"' \
  cargo build --manifest-path /workspace/testing-framework/tools/cfgsync-runtime/Cargo.toml --bin cfgsync-client

cp /workspace/target/debug/cfgsync-server /workspace/artifacts/cfgsync-server
cp /workspace/target/debug/cfgsync-client /workspace/artifacts/cfgsync-client

rm -rf /workspace/target/debug/incremental
