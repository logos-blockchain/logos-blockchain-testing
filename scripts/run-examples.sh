#!/usr/bin/env bash
set -euo pipefail

# All-in-one helper: prepare circuits (Linux + host), rebuild the image, and run
# the chosen runner binary.
#
# Usage: scripts/run-examples.sh [options] [compose|host|k8s]
#   compose -> runs examples/src/bin/compose_runner.rs (default)
#   host    -> runs examples/src/bin/local_runner.rs
#   k8s     -> runs examples/src/bin/k8s_runner.rs
#   run-seconds must be provided via -t/--run-seconds
#
# Env overrides:
#   VERSION                       - circuits version (default v0.3.1)
#   NOMOS_TESTNET_IMAGE           - image tag (default nomos-testnet:local)
#   NOMOS_CIRCUITS_PLATFORM       - override host platform detection
#   NOMOS_CIRCUITS_REBUILD_RAPIDSNARK - set to 1 to force rapidsnark rebuild
#   NOMOS_NODE_REV                - nomos-node git rev for local binaries (default d2dd5a5084e1daef4032562c77d41de5e4d495f8)

usage() {
  cat <<'EOF'
Usage: scripts/run-examples.sh [options] [compose|host|k8s]

Modes:
  compose   Run examples/src/bin/compose_runner.rs (default)
  host      Run examples/src/bin/local_runner.rs
  k8s       Run examples/src/bin/k8s_runner.rs

Options:
  -t, --run-seconds N   Duration to run the demo (required)
  -v, --validators N    Number of validators (required)
  -e, --executors N     Number of executors (required)

Environment:
  VERSION                        Circuits version (default v0.3.1)
  NOMOS_TESTNET_IMAGE            Image tag (default nomos-testnet:local)
  NOMOS_CIRCUITS_PLATFORM        Override host platform detection
  NOMOS_CIRCUITS_REBUILD_RAPIDSNARK  Force rapidsnark rebuild
  NOMOS_NODE_REV                 nomos-node git rev (default d2dd5a5084e1daef4032562c77d41de5e4d495f8)
  NOMOS_BINARIES_TAR             Path to prebuilt binaries/circuits tarball
  NOMOS_SKIP_IMAGE_BUILD         Set to 1 to skip rebuilding the compose/k8s image
EOF
}

fail_with_usage() {
  echo "$1" >&2
  usage
  exit 1
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

readonly ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly DEFAULT_VERSION="v0.3.1"
readonly DEFAULT_NODE_REV="d2dd5a5084e1daef4032562c77d41de5e4d495f8"
MODE="compose"
RUN_SECS_RAW=""
VERSION="${VERSION:-${DEFAULT_VERSION}}"
IMAGE="${NOMOS_TESTNET_IMAGE:-nomos-testnet:local}"
NOMOS_NODE_REV="${NOMOS_NODE_REV:-${DEFAULT_NODE_REV}}"
DEMO_VALIDATORS=""
DEMO_EXECUTORS=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage; exit 0 ;;
    -t|--run-seconds)
      RUN_SECS_RAW="${2:-}"; shift 2 ;;
    -v|--validators)
      DEMO_VALIDATORS="${2:-}"; shift 2 ;;
    -e|--executors)
      DEMO_EXECUTORS="${2:-}"; shift 2 ;;
    compose|host|k8s)
      MODE="$1"; shift ;;
    *)
      # Positional run-seconds fallback for legacy usage
      if [ -z "${RUN_SECS_RAW_SPECIFIED:-}" ] && [[ "$1" =~ ^[0-9]+$ ]]; then
        RUN_SECS_RAW="$1"
        shift
      else
        fail_with_usage "Unknown argument: $1"
      fi
      ;;
  esac
done
RESTORED_BINARIES=0
SETUP_OUT=""
cleanup() {
  if [ -n "${SETUP_OUT}" ]; then
    rm -f "${SETUP_OUT}"
  fi
}
trap cleanup EXIT

case "$MODE" in
  compose) BIN="compose_runner" ;;
  host) BIN="local_runner" ;;
  k8s) BIN="k8s_runner" ;;
  *) echo "Unknown mode '$MODE' (use compose|host|k8s)" >&2; exit 1 ;;
esac

if ! [[ "${RUN_SECS_RAW}" =~ ^[0-9]+$ ]] || [ "${RUN_SECS_RAW}" -le 0 ]; then
  fail_with_usage "run-seconds must be a positive integer (pass -t/--run-seconds)"
fi
readonly RUN_SECS="${RUN_SECS_RAW}"
if [ -n "${DEMO_VALIDATORS}" ] && ! [[ "${DEMO_VALIDATORS}" =~ ^[0-9]+$ ]] ; then
  fail_with_usage "validators must be a non-negative integer (pass -v/--validators)"
fi
if [ -n "${DEMO_EXECUTORS}" ] && ! [[ "${DEMO_EXECUTORS}" =~ ^[0-9]+$ ]] ; then
  fail_with_usage "executors must be a non-negative integer (pass -e/--executors)"
fi
if [ -z "${DEMO_VALIDATORS}" ] || [ -z "${DEMO_EXECUTORS}" ]; then
  fail_with_usage "validators and executors must be provided via -v/--validators and -e/--executors"
fi

default_tar_path() {
  # Pick a sensible default tarball based on mode and version.
  if [ -n "${NOMOS_BINARIES_TAR:-}" ]; then
    echo "${NOMOS_BINARIES_TAR}"
    return
  fi
  case "$MODE" in
    host) echo "${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz" ;;
    compose|k8s) echo "${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz" ;;
    *) echo "${ROOT_DIR}/.tmp/nomos-binaries-${VERSION}.tar.gz" ;;
  esac
}

restore_binaries_from_tar() {
  local tar_path
  tar_path="$(default_tar_path)"
  local extract_dir="${ROOT_DIR}/.tmp/nomos-binaries"
  if [ ! -f "$tar_path" ]; then
    return 1
  fi
  echo "==> Restoring binaries from ${tar_path}"
  rm -rf "${extract_dir}"
  mkdir -p "${extract_dir}"
  tar -xzf "$tar_path" -C "${extract_dir}"
  local src="${extract_dir}/artifacts"
  local bin_dst="${ROOT_DIR}/testing-framework/assets/stack/bin"
  local circuits_src="${src}/circuits"
  local circuits_dst="${ROOT_DIR}/testing-framework/assets/stack/kzgrs_test_params"
  if [ -f "${src}/nomos-node" ] && [ -f "${src}/nomos-executor" ] && [ -f "${src}/nomos-cli" ]; then
    mkdir -p "${bin_dst}"
    cp "${src}/nomos-node" "${src}/nomos-executor" "${src}/nomos-cli" "${bin_dst}/"
  else
    echo "Binaries missing in ${tar_path}; fallback to build-from-source path (run build-binaries workflow to populate)" >&2
    return 1
  fi
  if [ -d "${circuits_src}" ] && [ -f "${circuits_src}/kzgrs_test_params" ]; then
    rm -rf "${circuits_dst}"
    mkdir -p "${circuits_dst}"
    if command -v rsync >/dev/null 2>&1; then
      rsync -a --delete "${circuits_src}/" "${circuits_dst}/"
    else
      rm -rf "${circuits_dst:?}/"*
      cp -a "${circuits_src}/." "${circuits_dst}/"
    fi
  else
    echo "Circuits missing in ${tar_path}; fallback to download/build path (run build-binaries workflow to populate)" >&2
    return 1
  fi
  RESTORED_BINARIES=1
  export RESTORED_BINARIES
}

host_bin_matches_arch() {
  local bin_path="$1"
  if [ ! -x "$bin_path" ]; then
    return 1
  fi
  local info expected
  info="$(file -b "$bin_path" 2>/dev/null || true)"
  case "$(uname -m)" in
    x86_64) expected="x86-64|x86_64" ;;
    aarch64|arm64) expected="arm64|aarch64" ;;
    *) expected="" ;;
  esac
  if [ -n "$expected" ] && echo "$info" | grep -Eqi "$expected"; then
    return 0
  fi
  return 1
}

ensure_host_binaries() {
  # Build nomos-node/nomos-executor for the host if not already present.
  HOST_SRC="${ROOT_DIR}/.tmp/nomos-node-host-src"
  HOST_TARGET="${ROOT_DIR}/.tmp/nomos-node-host-target"
  HOST_NODE_BIN_DEFAULT="${HOST_TARGET}/debug/nomos-node"
  HOST_EXEC_BIN_DEFAULT="${HOST_TARGET}/debug/nomos-executor"
  HOST_ASSET_NODE_BIN="${ROOT_DIR}/testing-framework/assets/stack/bin/nomos-node"
  HOST_ASSET_EXEC_BIN="${ROOT_DIR}/testing-framework/assets/stack/bin/nomos-executor"

  if [ -n "${NOMOS_NODE_BIN:-}" ] && [ -x "${NOMOS_NODE_BIN}" ] && [ -x "${NOMOS_EXECUTOR_BIN:-}" ]; then
    if host_bin_matches_arch "${NOMOS_NODE_BIN}"; then
      echo "Using provided host binaries:"
      echo "  NOMOS_NODE_BIN=${NOMOS_NODE_BIN}"
      echo "  NOMOS_EXECUTOR_BIN=${NOMOS_EXECUTOR_BIN}"
      return
    else
      echo "Provided NOMOS_NODE_BIN does not match host arch; rebuilding..."
    fi
  fi

  if host_bin_matches_arch "${HOST_NODE_BIN_DEFAULT}" && host_bin_matches_arch "${HOST_EXEC_BIN_DEFAULT}"; then
    echo "Host binaries already built at ${HOST_TARGET}"
    NOMOS_NODE_BIN="${HOST_NODE_BIN_DEFAULT}"
    NOMOS_EXECUTOR_BIN="${HOST_EXEC_BIN_DEFAULT}"
    export NOMOS_NODE_BIN NOMOS_EXECUTOR_BIN
    return
  fi

  if [ "${RESTORED_BINARIES}" -eq 1 ] && host_bin_matches_arch "${HOST_ASSET_NODE_BIN}" && host_bin_matches_arch "${HOST_ASSET_EXEC_BIN}"; then
    echo "Using restored host binaries from bundle"
    NOMOS_NODE_BIN="${HOST_ASSET_NODE_BIN}"
    NOMOS_EXECUTOR_BIN="${HOST_ASSET_EXEC_BIN}"
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
    echo "-> Compiling host binaries (may take a few minutes)..."
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

restore_binaries_from_tar || true

echo "==> Preparing circuits (version ${VERSION})"
SETUP_OUT="$(mktemp -t nomos-setup-output.XXXXXX)"
if [ "${RESTORED_BINARIES}" -ne 1 ]; then
  "${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}" </dev/null | tee "$SETUP_OUT"
else
  echo "Skipping circuits setup; using restored bundle"
fi

# When running compose/k8s on macOS, prefer host-installed circuits so the
# host-side zksign tooling matches the host architecture even if the bundle was
# restored from a linux tarball.
if [ "${RESTORED_BINARIES}" -eq 1 ] && [ "$MODE" != "host" ] && [ "$(uname -s)" != "Linux" ]; then
  "${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}" </dev/null | tee -a "$SETUP_OUT"
fi

# Prefer host-native bundle for host tooling when available; otherwise fall back
# to the restored circuits location.
if [ "$(uname -s)" != "Linux" ] && [ -d "${ROOT_DIR}/.tmp/nomos-circuits-host" ]; then
  HOST_BUNDLE_PATH="${ROOT_DIR}/.tmp/nomos-circuits-host"
elif [ "${RESTORED_BINARIES}" -eq 1 ]; then
  HOST_BUNDLE_PATH="${ROOT_DIR}/testing-framework/assets/stack/kzgrs_test_params"
else
  HOST_BUNDLE_PATH="${ROOT_DIR}/testing-framework/assets/stack/kzgrs_test_params"
fi

# If the host bundle was somehow pruned, repair it once more.
if [ ! -x "${HOST_BUNDLE_PATH}/zksign/witness_generator" ]; then
  echo "Host circuits missing zksign/witness_generator; repairing..."
  "${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}"
fi
KZG_HOST_PATH="${HOST_BUNDLE_PATH}/kzgrs_test_params"
if [ ! -f "${KZG_HOST_PATH}" ]; then
  echo "KZG params missing at ${KZG_HOST_PATH}; rebuilding circuits bundle"
  "${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}"
fi

if [ "$MODE" != "host" ]; then
  if [ "${RESTORED_BINARIES}" -ne 1 ]; then
    echo "WARNING: NOMOS_BINARIES_TAR not restored; compose/k8s will rebuild binaries from source" >&2
  fi
  if [ "${NOMOS_SKIP_IMAGE_BUILD:-0}" = "1" ]; then
    echo "==> Skipping testnet image rebuild (NOMOS_SKIP_IMAGE_BUILD=1)"
  else
    echo "==> Rebuilding testnet image (${IMAGE})"
    IMAGE_TAG="${IMAGE}" "${ROOT_DIR}/testing-framework/assets/stack/scripts/build_test_image.sh"
  fi
fi

if [ "$MODE" = "host" ]; then
  if [ "${RESTORED_BINARIES}" -eq 1 ] && [ "$(uname -s)" = "Linux" ]; then
    tar_node="${ROOT_DIR}/testing-framework/assets/stack/bin/nomos-node"
    tar_exec="${ROOT_DIR}/testing-framework/assets/stack/bin/nomos-executor"
    if [ -x "${tar_node}" ] && [ -x "${tar_exec}" ]; then
      echo "==> Using restored host binaries from tarball"
      NOMOS_NODE_BIN="${tar_node}"
      NOMOS_EXECUTOR_BIN="${tar_exec}"
      export NOMOS_NODE_BIN NOMOS_EXECUTOR_BIN
    else
      echo "Restored tarball missing executables for host; building host binaries..."
      ensure_host_binaries
    fi
  else
    ensure_host_binaries
  fi
fi

echo "==> Running ${BIN} for ${RUN_SECS}s"
cd "${ROOT_DIR}"
if [ "$MODE" = "compose" ] || [ "$MODE" = "k8s" ]; then
  KZG_PATH="/kzgrs_test_params/kzgrs_test_params"
else
  KZG_PATH="${KZG_HOST_PATH}"
fi

# Ensure compose image pulls circuits for the host architecture by default.
if [ "$MODE" = "compose" ] && [ -z "${COMPOSE_CIRCUITS_PLATFORM:-}" ]; then
  arch="$(uname -m)"
  case "$arch" in
    x86_64) COMPOSE_CIRCUITS_PLATFORM="linux-x86_64" ;;
    arm64|aarch64) COMPOSE_CIRCUITS_PLATFORM="linux-aarch64" ;;
    *) COMPOSE_CIRCUITS_PLATFORM="linux-x86_64" ;;
  esac
fi

if [ -n "${DEMO_VALIDATORS}" ]; then
  export NOMOS_DEMO_VALIDATORS="${DEMO_VALIDATORS}"
fi
if [ -n "${DEMO_EXECUTORS}" ]; then
  export NOMOS_DEMO_EXECUTORS="${DEMO_EXECUTORS}"
fi
POL_PROOF_DEV_MODE=true \
NOMOS_TESTNET_IMAGE="${IMAGE}" \
NOMOS_CIRCUITS="${HOST_BUNDLE_PATH}" \
NOMOS_KZGRS_PARAMS_PATH="${KZG_PATH}" \
COMPOSE_DEMO_RUN_SECS="${RUN_SECS}" \
LOCAL_DEMO_RUN_SECS="${RUN_SECS}" \
K8S_DEMO_RUN_SECS="${RUN_SECS}" \
NOMOS_NODE_BIN="${NOMOS_NODE_BIN:-}" \
NOMOS_EXECUTOR_BIN="${NOMOS_EXECUTOR_BIN:-}" \
COMPOSE_CIRCUITS_PLATFORM="${COMPOSE_CIRCUITS_PLATFORM:-}" \
  cargo run -p runner-examples --bin "${BIN}"
