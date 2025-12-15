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
#   NOMOS_TESTNET_IMAGE           - image tag (default logos-blockchain-testing:local)
#   NOMOS_CIRCUITS_PLATFORM       - override host platform detection
#   NOMOS_CIRCUITS_REBUILD_RAPIDSNARK - set to 1 to force rapidsnark rebuild
#   NOMOS_BINARIES_TAR            - path to prebuilt binaries/circuits tarball (required; default .tmp/nomos-binaries-<mode>-<version>.tar.gz)

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
  --no-image-build      Skip rebuilding the compose/k8s image (sets NOMOS_SKIP_IMAGE_BUILD=1)

Environment:
  VERSION                        Circuits version (default v0.3.1)
  NOMOS_TESTNET_IMAGE            Image tag (default logos-blockchain-testing:local)
  NOMOS_CIRCUITS_PLATFORM        Override host platform detection
  NOMOS_CIRCUITS_REBUILD_RAPIDSNARK  Force rapidsnark rebuild
  NOMOS_BINARIES_TAR             Path to prebuilt binaries/circuits tarball (required)
  NOMOS_SKIP_IMAGE_BUILD         Set to 1 to skip rebuilding the compose/k8s image
  TESTNET_PRINT_ENDPOINTS        If set, runners print TESTNET_ENDPOINTS/TESTNET_PPROF (set automatically)
  COMPOSE_RUNNER_HTTP_TIMEOUT_SECS  Compose readiness timeout override
  K8S_RUNNER_DEPLOYMENT_TIMEOUT_SECS  K8s deployment readiness timeout override
  K8S_RUNNER_HTTP_TIMEOUT_SECS      K8s port-forward readiness timeout override
  K8S_RUNNER_HTTP_PROBE_TIMEOUT_SECS K8s NodePort readiness timeout override
  K8S_RUNNER_PROMETHEUS_HTTP_TIMEOUT_SECS        K8s Prometheus port-forward readiness timeout override
  K8S_RUNNER_PROMETHEUS_HTTP_PROBE_TIMEOUT_SECS  K8s Prometheus NodePort probe timeout override
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

# If a tarball is explicitly provided, ensure it exists before doing work.
if [ -n "${NOMOS_BINARIES_TAR:-}" ] && [ ! -f "${NOMOS_BINARIES_TAR}" ]; then
  fail_with_usage "NOMOS_BINARIES_TAR is set but missing: ${NOMOS_BINARIES_TAR}"
fi

readonly ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ ! -f "${ROOT_DIR}/versions.env" ]; then
  echo "ERROR: versions.env missing; run from repo root or restore the file." >&2
  exit 1
fi
# shellcheck disable=SC1091
. "${ROOT_DIR}/versions.env"
if [ -f "${ROOT_DIR}/paths.env" ]; then
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/paths.env"
fi
readonly DEFAULT_VERSION="${VERSION:?Missing VERSION in versions.env}"
readonly KZG_DIR_REL="${NOMOS_KZG_DIR_REL:-testing-framework/assets/stack/kzgrs_test_params}"
readonly KZG_FILE="${NOMOS_KZG_FILE:-kzgrs_test_params}"
readonly KZG_CONTAINER_PATH="${NOMOS_KZG_CONTAINER_PATH:-/kzgrs_test_params/kzgrs_test_params}"
readonly HOST_KZG_DIR="${ROOT_DIR}/${KZG_DIR_REL}"
readonly HOST_KZG_FILE="${HOST_KZG_DIR}/${KZG_FILE}"
MODE="compose"
RUN_SECS_RAW=""
VERSION="${DEFAULT_VERSION}"
IMAGE="${NOMOS_TESTNET_IMAGE:-logos-blockchain-testing:local}"
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
    --no-image-build)
      NOMOS_SKIP_IMAGE_BUILD=1
      export NOMOS_SKIP_IMAGE_BUILD
      shift ;;
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
    host)
      echo "${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz"
      ;;
    compose|k8s)
      # When skipping image rebuild, we need host-arch tools (witness generators) on the runner.
      if [ "${NOMOS_SKIP_IMAGE_BUILD:-}" = "1" ]; then
        echo "${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz"
      else
        echo "${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"
      fi
      ;;
    *) echo "${ROOT_DIR}/.tmp/nomos-binaries-${VERSION}.tar.gz" ;;
  esac
}

restore_binaries_from_tar() {
  local tar_path
  if [ -n "${_RESTORE_TAR_OVERRIDE:-}" ]; then
    tar_path="${_RESTORE_TAR_OVERRIDE}"
  else
    tar_path="$(default_tar_path)"
  fi
  if ! bundle_matches_expected "${tar_path}"; then
    return 1
  fi
  local extract_dir="${ROOT_DIR}/.tmp/nomos-binaries"
  if [ ! -f "$tar_path" ]; then
    return 1
  fi
  echo "==> Restoring binaries from ${tar_path}"
  rm -rf "${extract_dir}"
  mkdir -p "${extract_dir}"
  if ! tar -xzf "$tar_path" -C "${extract_dir}"; then
    echo "Failed to extract ${tar_path}" >&2
    return 1
  fi
  local src="${extract_dir}/artifacts"
  local bin_dst="${ROOT_DIR}/testing-framework/assets/stack/bin"
  local circuits_src="${src}/circuits"
  local circuits_dst="${HOST_KZG_DIR}"
  RESTORED_BIN_DIR="${src}"
  export RESTORED_BIN_DIR
  if [ -f "${src}/nomos-node" ] && [ -f "${src}/nomos-executor" ] && [ -f "${src}/nomos-cli" ]; then
    local copy_bins=1
    if [ "$MODE" != "host" ] && ! host_bin_matches_arch "${src}/nomos-node"; then
      echo "Bundled binaries do not match host arch; skipping copy so containers rebuild from source."
      copy_bins=0
      rm -f "${bin_dst}/nomos-node" "${bin_dst}/nomos-executor" "${bin_dst}/nomos-cli"
    fi
    if [ "$copy_bins" -eq 1 ]; then
      mkdir -p "${bin_dst}"
      cp "${src}/nomos-node" "${src}/nomos-executor" "${src}/nomos-cli" "${bin_dst}/"
    fi
  else
    echo "Binaries missing in ${tar_path}; provide a prebuilt binaries tarball." >&2
    return 1
  fi
  if [ -d "${circuits_src}" ] && [ -f "${circuits_src}/${KZG_FILE}" ]; then
    rm -rf "${circuits_dst}"
    mkdir -p "${circuits_dst}"
    if command -v rsync >/dev/null 2>&1; then
      rsync -a --delete "${circuits_src}/" "${circuits_dst}/"
    else
      rm -rf "${circuits_dst:?}/"*
      cp -a "${circuits_src}/." "${circuits_dst}/"
    fi
  else
    echo "Circuits missing in ${tar_path}; provide a prebuilt binaries/circuits tarball." >&2
    return 1
  fi
  RESTORED_BINARIES=1
  export RESTORED_BINARIES
}

bundle_matches_expected() {
  local tar_path="$1"
  if [ ! -f "${tar_path}" ]; then
    return 1
  fi
  if [ -z "${NOMOS_NODE_REV:-}" ]; then
    return 0
  fi
  local meta
  meta="$(tar -xOzf "${tar_path}" artifacts/nomos-bundle-meta.env 2>/dev/null || true)"
  if [ -z "${meta}" ]; then
    echo "Bundle meta missing in ${tar_path}; treating as stale and rebuilding." >&2
    return 1
  fi
  local tar_rev tar_head
  tar_rev="$(echo "${meta}" | sed -n 's/^nomos_node_rev=//p' | head -n 1)"
  tar_head="$(echo "${meta}" | sed -n 's/^nomos_node_git_head=//p' | head -n 1)"
  if [ -n "${tar_rev}" ] && [ "${tar_rev}" != "${NOMOS_NODE_REV}" ]; then
    echo "Bundle ${tar_path} is for nomos-node rev ${tar_rev}, expected ${NOMOS_NODE_REV}; rebuilding." >&2
    return 1
  fi
  if [ -n "${tar_head}" ] && [ "${tar_head}" != "${NOMOS_NODE_REV}" ]; then
    echo "Bundle ${tar_path} is for nomos-node git head ${tar_head}, expected ${NOMOS_NODE_REV}; rebuilding." >&2
    return 1
  fi
  return 0
}

ensure_binaries_tar() {
  local platform="$1"
  local tar_path="$2"
  echo "==> Building fresh binaries bundle (${platform}) at ${tar_path}"
  "${ROOT_DIR}/scripts/build-bundle.sh" --platform "${platform}" --output "${tar_path}" --rev "${NOMOS_NODE_REV}"
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

HOST_TAR="${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz"
LINUX_TAR="${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"
NEED_HOST_RESTORE_AFTER_IMAGE=0

if [ -n "${NOMOS_NODE_BIN:-}" ] && [ -x "${NOMOS_NODE_BIN}" ] && [ -n "${NOMOS_EXECUTOR_BIN:-}" ] && [ -x "${NOMOS_EXECUTOR_BIN}" ]; then
  echo "==> Using pre-specified host binaries (NOMOS_NODE_BIN/NOMOS_EXECUTOR_BIN); skipping tarball restore"
else
  # On non-Linux compose/k8s runs, use the Linux bundle for image build, then restore host bundle for the runner.
  if [ "$MODE" != "host" ] && [ "$(uname -s)" != "Linux" ] && [ "${NOMOS_SKIP_IMAGE_BUILD:-0}" = "0" ] && [ -f "${LINUX_TAR}" ]; then
    NEED_HOST_RESTORE_AFTER_IMAGE=1
    _RESTORE_TAR_OVERRIDE="${LINUX_TAR}" restore_binaries_from_tar || {
      ensure_binaries_tar linux "${LINUX_TAR}"
      _RESTORE_TAR_OVERRIDE="${LINUX_TAR}" restore_binaries_from_tar
    }
    unset _RESTORE_TAR_OVERRIDE
  fi

  if ! restore_binaries_from_tar; then
    tar_path="$(default_tar_path)"
    case "$MODE" in
      host) ensure_binaries_tar host "${tar_path}" ;;
      compose|k8s)
        if [ "${NOMOS_SKIP_IMAGE_BUILD:-0}" = "1" ]; then
          ensure_binaries_tar host "${tar_path}"
        else
          ensure_binaries_tar linux "${tar_path}"
        fi
        ;;
      *) ensure_binaries_tar host "${tar_path}" ;;
    esac
    restore_binaries_from_tar || {
      echo "ERROR: Missing or invalid binaries tarball. Provide it via NOMOS_BINARIES_TAR or place it at $(default_tar_path)." >&2
      exit 1
    }
  fi
fi

echo "==> Using restored circuits/binaries bundle"
SETUP_OUT="$(mktemp -t nomos-setup-output.XXXXXX)"
if [ "$MODE" != "host" ]; then
  if [ "${NOMOS_SKIP_IMAGE_BUILD:-0}" = "1" ]; then
    echo "==> Skipping testnet image rebuild (NOMOS_SKIP_IMAGE_BUILD=1)"
  else
    echo "==> Rebuilding testnet image (${IMAGE})"
    IMAGE_TAG="${IMAGE}" COMPOSE_CIRCUITS_PLATFORM="${COMPOSE_CIRCUITS_PLATFORM:-}" \
      "${ROOT_DIR}/testing-framework/assets/stack/scripts/build_test_image.sh"
  fi
fi

if [ "${NEED_HOST_RESTORE_AFTER_IMAGE}" = "1" ]; then
  if [ -f "${HOST_TAR}" ]; then
    echo "==> Restoring host bundle for runner (${HOST_TAR})"
    _RESTORE_TAR_OVERRIDE="${HOST_TAR}" restore_binaries_from_tar || {
      ensure_binaries_tar host "${HOST_TAR}"
      _RESTORE_TAR_OVERRIDE="${HOST_TAR}" restore_binaries_from_tar || {
        echo "ERROR: Failed to restore host bundle from ${HOST_TAR}" >&2
        exit 1
      }
    }
    unset _RESTORE_TAR_OVERRIDE
    echo "==> Using restored circuits/binaries bundle"
  else
    ensure_binaries_tar host "${HOST_TAR}"
    _RESTORE_TAR_OVERRIDE="${HOST_TAR}" restore_binaries_from_tar || {
      echo "ERROR: Failed to restore host bundle from ${HOST_TAR}" >&2
      exit 1
    }
    unset _RESTORE_TAR_OVERRIDE
    echo "==> Using restored circuits/binaries bundle"
  fi
fi

HOST_BUNDLE_PATH="${HOST_KZG_DIR}"

# If the host bundle was somehow pruned, repair it once more.
if [ ! -x "${HOST_BUNDLE_PATH}/zksign/witness_generator" ]; then
  echo "ERROR: Missing zksign/witness_generator in restored bundle; ensure the tarball contains host-compatible circuits." >&2
  exit 1
fi
KZG_HOST_PATH="${HOST_BUNDLE_PATH}/${KZG_FILE}"
if [ ! -f "${KZG_HOST_PATH}" ]; then
  echo "ERROR: KZG params missing at ${KZG_HOST_PATH}; ensure the tarball contains circuits." >&2
  exit 1
fi

if [ "$MODE" = "host" ]; then
  if [ -n "${NOMOS_NODE_BIN:-}" ] && [ -x "${NOMOS_NODE_BIN}" ] && [ -n "${NOMOS_EXECUTOR_BIN:-}" ] && [ -x "${NOMOS_EXECUTOR_BIN}" ]; then
    echo "==> Using provided host binaries (env override)"
  else
    tar_node="${RESTORED_BIN_DIR:-${ROOT_DIR}/testing-framework/assets/stack/bin}/nomos-node"
    tar_exec="${RESTORED_BIN_DIR:-${ROOT_DIR}/testing-framework/assets/stack/bin}/nomos-executor"
  if [ ! -x "${tar_node}" ] || [ ! -x "${tar_exec}" ]; then
    echo "ERROR: Restored tarball missing host executables; provide a host-compatible binaries tarball." >&2
    exit 1
  fi
  if ! host_bin_matches_arch "${tar_node}" || ! host_bin_matches_arch "${tar_exec}"; then
    echo "ERROR: Restored executables do not match host architecture; provide a host-compatible binaries tarball." >&2
    exit 1
  fi
    echo "==> Using restored host binaries from tarball"
    NOMOS_NODE_BIN="${tar_node}"
    NOMOS_EXECUTOR_BIN="${tar_exec}"
    export NOMOS_NODE_BIN NOMOS_EXECUTOR_BIN
  fi
fi

echo "==> Running ${BIN} for ${RUN_SECS}s"
cd "${ROOT_DIR}"
if [ "$MODE" = "compose" ] || [ "$MODE" = "k8s" ]; then
  KZG_PATH="${KZG_CONTAINER_PATH}"
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

export NOMOS_DEMO_RUN_SECS="${RUN_SECS}"

if [ -n "${DEMO_VALIDATORS}" ]; then
  export NOMOS_DEMO_VALIDATORS="${DEMO_VALIDATORS}"
fi
if [ -n "${DEMO_EXECUTORS}" ]; then
  export NOMOS_DEMO_EXECUTORS="${DEMO_EXECUTORS}"
fi
POL_PROOF_DEV_MODE=true \
TESTNET_PRINT_ENDPOINTS=1 \
NOMOS_TESTNET_IMAGE="${IMAGE}" \
NOMOS_CIRCUITS="${HOST_BUNDLE_PATH}" \
NOMOS_KZGRS_PARAMS_PATH="${KZG_PATH}" \
NOMOS_NODE_BIN="${NOMOS_NODE_BIN:-}" \
NOMOS_EXECUTOR_BIN="${NOMOS_EXECUTOR_BIN:-}" \
COMPOSE_CIRCUITS_PLATFORM="${COMPOSE_CIRCUITS_PLATFORM:-}" \
  cargo run -p runner-examples --bin "${BIN}"
