#!/usr/bin/env bash
set -euo pipefail

cargo build --all-features --manifest-path /workspace/testing-framework/tools/cfgsync/Cargo.toml --bins

cp /workspace/target/debug/cfgsync-server /workspace/artifacts/cfgsync-server
cp /workspace/target/debug/cfgsync-client /workspace/artifacts/cfgsync-client

rm -rf /workspace/target/debug/incremental

