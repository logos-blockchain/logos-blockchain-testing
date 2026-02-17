#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

echo "==> boundary check: lb-topology must stay local/topology-focused"

violations=0
topology_dir="${ROOT_DIR}/../nomos-node/tests/testing_framework/lb-topology"

if [ ! -d "${topology_dir}" ]; then
  echo "Boundary check failed: expected lb-topology at ${topology_dir}"
  exit 1
fi

echo "==> checking ${topology_dir}"

if rg -n "cfgsync|ComposeDeployEnv|K8sDeployEnv|runner-compose|runner-k8s|DEFAULT_CFGSYNC_PORT|DEFAULT_ASSETS_STACK_DIR" \
  "${topology_dir}/src" "${topology_dir}/Cargo.toml" >/tmp/lb-topology-boundary.out 2>&1; then
  echo "Boundary violation: extension-specific references found in lb-topology:"
  cat /tmp/lb-topology-boundary.out
  violations=1
fi

if [ "${violations}" -ne 0 ]; then
  exit 1
fi

echo "Boundary check passed."
