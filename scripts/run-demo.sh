#!/usr/bin/env bash
set -euo pipefail

# All-in-one helper: prepare circuits (Linux + host), rebuild the image, and run
# the chosen runner binary.
#
# Usage: scripts/run-demo.sh [compose|local|k8s] [run-seconds]
#   compose -> runs examples/src/bin/compose_runner.rs (default)
#   local   -> runs examples/src/bin/local_runner.rs
#   k8s     -> runs examples/src/bin/k8s_runner.rs
#   run-seconds defaults to 60
#
# Env overrides:
#   VERSION                       - circuits version (default v0.3.1)
#   NOMOS_TESTNET_IMAGE           - image tag (default nomos-testnet:local)
#   NOMOS_CIRCUITS_PLATFORM       - override host platform detection
#   NOMOS_CIRCUITS_REBUILD_RAPIDSNARK - set to 1 to force rapidsnark rebuild
#   NOMOS_NODE_REV                - nomos-node git rev for local binaries (default 2f60a0372c228968c3526c341ebc7e58bbd178dd)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-compose}"
RUN_SECS="${2:-60}"
VERSION="${VERSION:-v0.3.1}"
IMAGE="${NOMOS_TESTNET_IMAGE:-nomos-testnet:local}"
NOMOS_NODE_REV="${NOMOS_NODE_REV:-2f60a0372c228968c3526c341ebc7e58bbd178dd}"

case "$MODE" in
  compose) BIN="compose_runner" ;;
  local) BIN="local_runner" ;;
  k8s) BIN="k8s_runner" ;;
  *) echo "Unknown mode '$MODE' (use compose|local)" >&2; exit 1 ;;
esac

ensure_host_binaries() {
  # Build nomos-node/nomos-executor for the host if not already present.
  HOST_SRC="${ROOT_DIR}/.tmp/nomos-node-host-src"
  HOST_TARGET="${ROOT_DIR}/.tmp/nomos-node-host-target"
  HOST_NODE_BIN_DEFAULT="${HOST_TARGET}/debug/nomos-node"
  HOST_EXEC_BIN_DEFAULT="${HOST_TARGET}/debug/nomos-executor"

  if [ -n "${NOMOS_NODE_BIN:-}" ] && [ -x "${NOMOS_NODE_BIN}" ] && [ -x "${NOMOS_EXECUTOR_BIN:-}" ]; then
    echo "Using provided host binaries:"
    echo "  NOMOS_NODE_BIN=${NOMOS_NODE_BIN}"
    echo "  NOMOS_EXECUTOR_BIN=${NOMOS_EXECUTOR_BIN}"
    return
  fi

  if [ -x "${HOST_NODE_BIN_DEFAULT}" ] && [ -x "${HOST_EXEC_BIN_DEFAULT}" ]; then
    echo "Host binaries already built at ${HOST_TARGET}"
    NOMOS_NODE_BIN="${HOST_NODE_BIN_DEFAULT}"
    NOMOS_EXECUTOR_BIN="${HOST_EXEC_BIN_DEFAULT}"
    export NOMOS_NODE_BIN NOMOS_EXECUTOR_BIN
    return
  fi

  echo "Building host nomos-node/nomos-executor from ${NOMOS_NODE_REV}"
  mkdir -p "${HOST_SRC}"
  if [ ! -d "${HOST_SRC}/.git" ]; then
    git clone https://github.com/logos-co/nomos-node.git "${HOST_SRC}"
  fi
  (
    cd "${HOST_SRC}"
    git fetch --depth 1 origin "${NOMOS_NODE_REV}"
    git checkout "${NOMOS_NODE_REV}"
    git reset --hard
    git clean -fdx
    RUSTFLAGS='--cfg feature="pol-dev-mode"' \
      NOMOS_CIRCUITS="${HOST_BUNDLE_PATH}" \
      cargo build --features "testing" \
        -p nomos-node -p nomos-executor -p nomos-cli \
        --target-dir "${HOST_TARGET}"
  )
  NOMOS_NODE_BIN="${HOST_NODE_BIN_DEFAULT}"
  NOMOS_EXECUTOR_BIN="${HOST_EXEC_BIN_DEFAULT}"
  export NOMOS_NODE_BIN NOMOS_EXECUTOR_BIN
}

echo "==> Preparing circuits (version ${VERSION})"
SETUP_OUT="/tmp/nomos-setup-output.$$"
"${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}" </dev/null | tee "$SETUP_OUT"

# Prefer the host bundle if it exists; otherwise fall back to Linux bundle.
if [ -d "${ROOT_DIR}/.tmp/nomos-circuits-host" ]; then
  HOST_BUNDLE_PATH="${ROOT_DIR}/.tmp/nomos-circuits-host"
else
  HOST_BUNDLE_PATH="${ROOT_DIR}/testing-framework/assets/stack/kzgrs_test_params"
fi
rm -f "$SETUP_OUT"

# If the host bundle was somehow pruned, repair it once more.
if [ ! -x "${HOST_BUNDLE_PATH}/zksign/witness_generator" ]; then
  echo "Host circuits missing zksign/witness_generator; repairing..."
  "${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}"
fi

if [ "$MODE" != "local" ]; then
  echo "==> Rebuilding testnet image (${IMAGE})"
  "${ROOT_DIR}/testing-framework/assets/stack/scripts/build_test_image.sh"
fi

if [ "$MODE" = "local" ]; then
  ensure_host_binaries
fi

echo "==> Running ${BIN} for ${RUN_SECS}s"
cd "${ROOT_DIR}"
POL_PROOF_DEV_MODE=true \
NOMOS_TESTNET_IMAGE="${IMAGE}" \
NOMOS_CIRCUITS="${HOST_BUNDLE_PATH}" \
NOMOS_KZGRS_PARAMS_PATH="${HOST_BUNDLE_PATH}/kzgrs_test_params" \
COMPOSE_DEMO_RUN_SECS="${RUN_SECS}" \
LOCAL_DEMO_RUN_SECS="${RUN_SECS}" \
K8S_DEMO_RUN_SECS="${RUN_SECS}" \
NOMOS_NODE_BIN="${NOMOS_NODE_BIN:-}" \
NOMOS_EXECUTOR_BIN="${NOMOS_EXECUTOR_BIN:-}" \
  cargo run -p runner-examples --bin "${BIN}"
