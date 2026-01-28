#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../lib/common.sh"

readonly DEFAULT_LOCAL_IMAGE="logos-blockchain-testing:local"
readonly DEFAULT_PUBLIC_ECR_REGISTRY="public.ecr.aws/r4s5t9y4"
readonly DEFAULT_PUBLIC_ECR_REPO="logos/logos-blockchain"
readonly DEFAULT_PRIVATE_ECR_REPO="logos-blockchain-testing"
readonly DEFAULT_ECR_TAG="test"
readonly DEFAULT_PRIVATE_AWS_REGION="ap-southeast-2"

readonly DEFAULT_PULL_POLICY_LOCAL="IfNotPresent"
readonly DEFAULT_PULL_POLICY_ECR="Always"
readonly DOCKER_DESKTOP_CONTEXT="docker-desktop"
readonly DEFAULT_K8S_ECR_SKIP_IMAGE_BUILD="1"

run_examples::cleanup() {
  rm -f "${SETUP_OUT:-}" 2>/dev/null || true
}

# Avoid inheriting environment-provided EXIT traps (e.g., from BASH_ENV) that can
# reference missing functions and fail at script termination.
trap run_examples::cleanup EXIT

run_examples::usage() {
  cat <<EOF
Usage: scripts/run/run-examples.sh [options] [compose|host|k8s]

Modes:
  compose   Run examples/src/bin/compose_runner.rs (default)
  host      Run examples/src/bin/local_runner.rs
  k8s       Run examples/src/bin/k8s_runner.rs

Options:
  -t, --run-seconds N     Duration to run the demo (required)
  -n, --nodes N           Number of nodes (required)
  --bundle PATH           Convenience alias for setting LOGOS_BLOCKCHAIN_BINARIES_TAR=PATH
  --metrics-query-url URL         PromQL base URL the runner process can query (optional)
  --metrics-otlp-ingest-url URL   Full OTLP HTTP ingest URL for node metrics export (optional)
  --external-prometheus URL            Alias for --metrics-query-url
  --external-otlp-metrics-endpoint URL  Alias for --metrics-otlp-ingest-url
  --local                 Use a local Docker image tag (default for docker-desktop k8s)
  --no-image-build        Skip rebuilding the compose/k8s image (sets LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD=1)

Environment:
  VERSION                          Bundle version (default from versions.env)
  CONSENSUS_SLOT_TIME              Consensus slot duration in seconds (default 2)
  CONSENSUS_ACTIVE_SLOT_COEFF      Probability a slot is active (default 0.9); expected block interval ≈ slot_time / coeff
  LOGOS_BLOCKCHAIN_TESTNET_IMAGE              Image reference (overridden by --local/--ecr selection)
  ECR_IMAGE                        Full image reference for --ecr (overrides ECR_REGISTRY/ECR_REPO/TAG)
  ECR_REGISTRY                     Registry hostname for --ecr (default ${DEFAULT_PUBLIC_ECR_REGISTRY})
  ECR_REPO                         Repository path for --ecr (default ${DEFAULT_PUBLIC_ECR_REPO})
  TAG                              Tag for --ecr (default ${DEFAULT_ECR_TAG})
  LOGOS_BLOCKCHAIN_TESTNET_IMAGE_PULL_POLICY  K8s imagePullPolicy (default ${DEFAULT_PULL_POLICY_LOCAL}; set to ${DEFAULT_PULL_POLICY_ECR} for --ecr)
  LOGOS_BLOCKCHAIN_BINARIES_TAR               Path to prebuilt binaries tarball (default .tmp/nomos-binaries-<platform>-<version>.tar.gz)
  LOGOS_BLOCKCHAIN_CIRCUITS        Directory containing circuits assets (defaults to ~/.logos-blockchain-circuits)
  CARGO_FEATURE_BUILD_VERIFICATION_KEY  Build flag to embed Groth16 verification keys in node binaries (recommended for host)
  LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD           Set to 1 to skip rebuilding the compose/k8s image
  LOGOS_BLOCKCHAIN_FORCE_IMAGE_BUILD          Set to 1 to force image rebuild even for k8s ECR mode
  LOGOS_BLOCKCHAIN_METRICS_QUERY_URL           PromQL base URL for the runner process (optional)
  LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL     Full OTLP HTTP ingest URL for node metrics export (optional)
  LOGOS_BLOCKCHAIN_GRAFANA_URL                 Grafana base URL for printing/logging (optional)

Notes:
  - For k8s runs on non-docker-desktop clusters (e.g. EKS), a locally built Docker image is not
    visible to the cluster. By default, this script skips local image rebuilds in that case.
    If you need a custom image, run scripts/build/build_test_image.sh and push it to a registry the
    cluster can pull from, then set LOGOS_BLOCKCHAIN_TESTNET_IMAGE accordingly.
EOF
}

run_examples::fail_with_usage() {
  echo "$1" >&2
  run_examples::usage
  exit 1
}

run_examples::load_env() {
  ROOT_DIR="$(common::repo_root)"
  export ROOT_DIR

  common::require_file "${ROOT_DIR}/versions.env"
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/versions.env"
  common::maybe_source "${ROOT_DIR}/paths.env"

  DEFAULT_VERSION="${VERSION:?Missing VERSION in versions.env}"
  VERSION="${VERSION:-${DEFAULT_VERSION}}"

}

run_examples::select_bin() {
  case "${MODE}" in
    compose) BIN="compose_runner" ;;
    host) BIN="local_runner" ;;
    k8s) BIN="k8s_runner" ;;
    *) common::die "Unknown mode '${MODE}' (use compose|host|k8s)" ;;
  esac
}

run_examples::parse_args() {
  MODE="compose"
  RUN_SECS_RAW=""
  DEMO_NODES=""
  IMAGE_SELECTION_MODE="auto"
  METRICS_QUERY_URL=""
  METRICS_OTLP_INGEST_URL=""

  RUN_SECS_RAW_SPECIFIED=""

  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h|--help)
        run_examples::usage
        exit 0
        ;;
      -t|--run-seconds)
        RUN_SECS_RAW_SPECIFIED=1
        RUN_SECS_RAW="${2:-}"
        shift 2
        ;;
      --run-seconds=*)
        RUN_SECS_RAW_SPECIFIED=1
        RUN_SECS_RAW="${1#*=}"
        shift
        ;;
      -n|--nodes)
        DEMO_NODES="${2:-}"
        shift 2
        ;;
      --nodes=*)
        DEMO_NODES="${1#*=}"
        shift
        ;;
      --bundle)
        LOGOS_BLOCKCHAIN_BINARIES_TAR="${2:-}"
        export LOGOS_BLOCKCHAIN_BINARIES_TAR
        shift 2
        ;;
      --bundle=*)
        LOGOS_BLOCKCHAIN_BINARIES_TAR="${1#*=}"
        export LOGOS_BLOCKCHAIN_BINARIES_TAR
        shift
        ;;
      --metrics-query-url)
        METRICS_QUERY_URL="${2:-}"
        shift 2
        ;;
      --metrics-query-url=*)
        METRICS_QUERY_URL="${1#*=}"
        shift
        ;;
      --metrics-otlp-ingest-url)
        METRICS_OTLP_INGEST_URL="${2:-}"
        shift 2
        ;;
      --metrics-otlp-ingest-url=*)
        METRICS_OTLP_INGEST_URL="${1#*=}"
        shift
        ;;
      --external-prometheus)
        METRICS_QUERY_URL="${2:-}"
        shift 2
        ;;
      --external-prometheus=*)
        METRICS_QUERY_URL="${1#*=}"
        shift
        ;;
      --external-otlp-metrics-endpoint)
        METRICS_OTLP_INGEST_URL="${2:-}"
        shift 2
        ;;
      --external-otlp-metrics-endpoint=*)
        METRICS_OTLP_INGEST_URL="${1#*=}"
        shift
        ;;
      --local)
        IMAGE_SELECTION_MODE="local"
        shift
        ;;
      --no-image-build)
        LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD=1
        export LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD
        shift
        ;;
      compose|host|k8s)
        MODE="$1"
        shift
        ;;
      *)
        # Positional run-seconds fallback for legacy usage.
        if [ -z "${RUN_SECS_RAW_SPECIFIED}" ] && common::is_uint "$1"; then
          RUN_SECS_RAW="$1"
          shift
        else
          run_examples::fail_with_usage "Unknown argument: $1"
        fi
        ;;
    esac
  done

  if [ -n "${LOGOS_BLOCKCHAIN_BINARIES_TAR:-}" ] && [ ! -f "${LOGOS_BLOCKCHAIN_BINARIES_TAR}" ]; then
    run_examples::fail_with_usage "LOGOS_BLOCKCHAIN_BINARIES_TAR is set but missing: ${LOGOS_BLOCKCHAIN_BINARIES_TAR}"
  fi

  if ! common::is_uint "${RUN_SECS_RAW}" || [ "${RUN_SECS_RAW}" -le 0 ]; then
    run_examples::fail_with_usage "run-seconds must be a positive integer (pass -t/--run-seconds)"
  fi
  RUN_SECS="${RUN_SECS_RAW}"

  if [ -z "${DEMO_NODES}" ]; then
    run_examples::fail_with_usage "nodes must be provided via -n/--nodes"
  fi
  if ! common::is_uint "${DEMO_NODES}" ; then
    run_examples::fail_with_usage "nodes must be a non-negative integer (pass -n/--nodes)"
  fi
}

run_examples::select_image() {
  local selection="${IMAGE_SELECTION_MODE}"
  local context=""

  if [ "${selection}" = "auto" ]; then
    if [ "${MODE}" = "k8s" ] && command -v kubectl >/dev/null 2>&1; then
      context="$(kubectl config current-context 2>/dev/null || true)"
      if [ "${context}" = "${DOCKER_DESKTOP_CONTEXT}" ]; then
        selection="local"
      else
        selection="ecr"
      fi
    else
      selection="local"
    fi
  fi

  if [ "${selection}" = "local" ]; then
    IMAGE="${LOGOS_BLOCKCHAIN_TESTNET_IMAGE:-${DEFAULT_LOCAL_IMAGE}}"
    export LOGOS_BLOCKCHAIN_TESTNET_IMAGE_PULL_POLICY="${LOGOS_BLOCKCHAIN_TESTNET_IMAGE_PULL_POLICY:-${DEFAULT_PULL_POLICY_LOCAL}}"
  elif [ "${selection}" = "ecr" ]; then
    local tag="${TAG:-${DEFAULT_ECR_TAG}}"
    if [ -n "${ECR_IMAGE:-}" ]; then
      IMAGE="${ECR_IMAGE}"
    elif [ -n "${ECR_REGISTRY:-}" ]; then
      local registry="${ECR_REGISTRY}"
      local repo="${ECR_REPO:-${DEFAULT_PUBLIC_ECR_REPO}}"
      IMAGE="${registry}/${repo}:${tag}"
    elif [ -n "${AWS_ACCOUNT_ID:-}" ]; then
      local aws_region="${AWS_REGION:-${DEFAULT_PRIVATE_AWS_REGION}}"
      local aws_account_id="${AWS_ACCOUNT_ID}"
      local repo="${ECR_REPO:-${DEFAULT_PRIVATE_ECR_REPO}}"
      IMAGE="${aws_account_id}.dkr.ecr.${aws_region}.amazonaws.com/${repo}:${tag}"
    else
      local registry="${DEFAULT_PUBLIC_ECR_REGISTRY}"
      local repo="${ECR_REPO:-${DEFAULT_PUBLIC_ECR_REPO}}"
      IMAGE="${registry}/${repo}:${tag}"
    fi
    export LOGOS_BLOCKCHAIN_TESTNET_IMAGE_PULL_POLICY="${LOGOS_BLOCKCHAIN_TESTNET_IMAGE_PULL_POLICY:-${DEFAULT_PULL_POLICY_ECR}}"
  else
    run_examples::fail_with_usage "Unknown image selection mode: ${selection}"
  fi

  export LOGOS_BLOCKCHAIN_IMAGE_SELECTION="${selection}"
  export IMAGE_TAG="${IMAGE}"
  export LOGOS_BLOCKCHAIN_TESTNET_IMAGE="${IMAGE}"

  if [ "${MODE}" = "k8s" ] && [ "${selection}" = "ecr" ]; then
    # A locally built Docker image isn't visible to remote clusters (e.g. EKS). Default to
    # skipping the local rebuild, unless the user explicitly set LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD or
    # overrides via LOGOS_BLOCKCHAIN_FORCE_IMAGE_BUILD=1.
    if [ "${LOGOS_BLOCKCHAIN_FORCE_IMAGE_BUILD:-0}" != "1" ]; then
      LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD="${LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD:-${DEFAULT_K8S_ECR_SKIP_IMAGE_BUILD}}"
      export LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD
    fi
  fi
}

run_examples::default_tar_path() {
  if [ -n "${LOGOS_BLOCKCHAIN_BINARIES_TAR:-}" ]; then
    echo "${LOGOS_BLOCKCHAIN_BINARIES_TAR}"
    return
  fi
  case "${MODE}" in
    host) echo "${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz" ;;
    compose|k8s)
      if [ "${LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD:-}" = "1" ]; then
        echo "${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz"
      else
        echo "${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"
      fi
      ;;
    *) echo "${ROOT_DIR}/.tmp/nomos-binaries-${VERSION}.tar.gz" ;;
  esac
}

run_examples::bundle_matches_expected() {
  local tar_path="$1"
  [ -f "${tar_path}" ] || return 1
  [ -z "${LOGOS_BLOCKCHAIN_NODE_REV:-}" ] && return 0
  local expected_features="${RUN_EXAMPLES_EXPECTED_BUNDLE_FEATURES:-all,pol-dev-mode,verification-keys}"

  local meta tar_rev tar_head tar_features
  meta="$(tar -xOzf "${tar_path}" artifacts/nomos-bundle-meta.env 2>/dev/null || true)"
  if [ -z "${meta}" ]; then
    echo "Bundle meta missing in ${tar_path}; treating as stale and rebuilding." >&2
    return 1
  fi
  tar_rev="$(echo "${meta}" | sed -n 's/^nomos_node_rev=//p' | head -n 1)"
  tar_head="$(echo "${meta}" | sed -n 's/^nomos_node_git_head=//p' | head -n 1)"
  tar_features="$(echo "${meta}" | sed -n 's/^features=//p' | head -n 1)"
  if [ -n "${expected_features}" ] && [ "${tar_features}" != "${expected_features}" ]; then
    echo "Bundle ${tar_path} features '${tar_features}' do not match expected '${expected_features}'; rebuilding." >&2
    return 1
  fi
  if [ -n "${tar_rev}" ] && [ "${tar_rev}" != "${LOGOS_BLOCKCHAIN_NODE_REV}" ]; then
    echo "Bundle ${tar_path} is for logos-blockchain-node rev ${tar_rev}, expected ${LOGOS_BLOCKCHAIN_NODE_REV}; rebuilding." >&2
    return 1
  fi
  if [ -n "${tar_head}" ] && echo "${LOGOS_BLOCKCHAIN_NODE_REV}" | grep -Eq '^[0-9a-f]{7,40}$'; then
    if [ "${tar_head}" != "${LOGOS_BLOCKCHAIN_NODE_REV}" ]; then
      echo "Bundle ${tar_path} is for logos-blockchain-node git head ${tar_head}, expected ${LOGOS_BLOCKCHAIN_NODE_REV}; rebuilding." >&2
      return 1
    fi
  fi
  return 0
}

run_examples::host_bin_matches_arch() {
  local bin_path="$1"
  [ -x "${bin_path}" ] || return 1
  command -v file >/dev/null 2>&1 || return 0

  local info expected
  info="$(file -b "${bin_path}" 2>/dev/null || true)"
  case "$(uname -m)" in
    x86_64) expected="x86-64|x86_64" ;;
    aarch64|arm64) expected="arm64|aarch64" ;;
    *) expected="" ;;
  esac
  [ -n "${expected}" ] && echo "${info}" | grep -Eqi "${expected}"
}

run_examples::restore_binaries_from_tar() {
  local tar_path="${1:-}"
  if [ -z "${tar_path}" ]; then
    tar_path="$(run_examples::default_tar_path)"
  fi
  run_examples::bundle_matches_expected "${tar_path}" || return 1
  [ -f "${tar_path}" ] || return 1

  local extract_dir="${ROOT_DIR}/.tmp/nomos-binaries"
  echo "==> Restoring binaries from ${tar_path}"
  rm -rf "${extract_dir}"
  mkdir -p "${extract_dir}"
  tar -xzf "${tar_path}" -C "${extract_dir}" || common::die "Failed to extract ${tar_path}"

  local src="${extract_dir}/artifacts"
  local bin_dst="${ROOT_DIR}/testing-framework/assets/stack/bin"
  RESTORED_BIN_DIR="${src}"
  export RESTORED_BIN_DIR

  if [ ! -f "${src}/logos-blockchain-node" ]; then
    echo "Binaries missing in ${tar_path}; provide a prebuilt binaries tarball." >&2
    return 1
  fi

  local copy_bins=1
  if [ "${MODE}" != "host" ] && ! run_examples::host_bin_matches_arch "${src}/logos-blockchain-node"; then
    echo "Bundled binaries do not match host arch; skipping copy so containers rebuild from source."
    copy_bins=0
    rm -f "${bin_dst}/logos-blockchain-node"
  fi
  if [ "${copy_bins}" -eq 1 ]; then
    mkdir -p "${bin_dst}"
    cp "${src}/logos-blockchain-node" "${bin_dst}/"
  fi

  RESTORED_BINARIES=1
  export RESTORED_BINARIES
}

run_examples::ensure_binaries_tar() {
  local platform="$1"
  local tar_path="$2"
  echo "==> Building fresh binaries bundle (${platform}) at ${tar_path}"
  "${ROOT_DIR}/scripts/build/build-bundle.sh" --platform "${platform}" --output "${tar_path}" --rev "${LOGOS_BLOCKCHAIN_NODE_REV}"
}

run_examples::prepare_bundles() {
  RESTORED_BINARIES=0
  NEED_HOST_RESTORE_AFTER_IMAGE=0

  HOST_TAR="${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz"
  LINUX_TAR="${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"

  if [ -n "${LOGOS_BLOCKCHAIN_NODE_BIN:-}" ] && [ -x "${LOGOS_BLOCKCHAIN_NODE_BIN}" ]; then
    echo "==> Using pre-specified host binaries (LOGOS_BLOCKCHAIN_NODE_BIN); skipping tarball restore"
    return 0
  fi

  # On non-Linux compose/k8s runs, use the Linux bundle for image build, then restore host bundle for the runner.
  if [ "${MODE}" != "host" ] && [ "$(uname -s)" != "Linux" ] && [ "${LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD:-0}" = "0" ] && [ -f "${LINUX_TAR}" ]; then
    NEED_HOST_RESTORE_AFTER_IMAGE=1
    run_examples::restore_binaries_from_tar "${LINUX_TAR}" || {
      run_examples::ensure_binaries_tar linux "${LINUX_TAR}"
      run_examples::restore_binaries_from_tar "${LINUX_TAR}"
    }
  fi

  if ! run_examples::restore_binaries_from_tar; then
    local tar_path
    tar_path="$(run_examples::default_tar_path)"
    case "${MODE}" in
      host) run_examples::ensure_binaries_tar host "${tar_path}" ;;
      compose|k8s)
        if [ "${LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD:-0}" = "1" ]; then
          run_examples::ensure_binaries_tar host "${tar_path}"
        else
          run_examples::ensure_binaries_tar linux "${tar_path}"
        fi
        ;;
      *) run_examples::ensure_binaries_tar host "${tar_path}" ;;
    esac

    run_examples::restore_binaries_from_tar "${tar_path}" || common::die \
      "Missing or invalid binaries tarball. Provide it via --bundle/LOGOS_BLOCKCHAIN_BINARIES_TAR or place it at $(run_examples::default_tar_path)."
  fi
}

run_examples::maybe_rebuild_image() {
  if [ "${MODE}" = "host" ]; then
    return 0
  fi

  if [ "${LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD:-0}" = "1" ]; then
    echo "==> Skipping testnet image rebuild (LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD=1)"
    return 0
  fi

  echo "==> Rebuilding testnet image (${IMAGE})"
  IMAGE_TAG="${IMAGE}" bash "${ROOT_DIR}/scripts/build/build_test_image.sh"
}

run_examples::maybe_restore_host_after_image() {
  if [ "${NEED_HOST_RESTORE_AFTER_IMAGE}" != "1" ]; then
    return 0
  fi

  echo "==> Restoring host bundle for runner (${HOST_TAR})"
  if [ ! -f "${HOST_TAR}" ]; then
    run_examples::ensure_binaries_tar host "${HOST_TAR}"
  fi
  run_examples::restore_binaries_from_tar "${HOST_TAR}" || common::die "Failed to restore host bundle from ${HOST_TAR}"
}

run_examples::validate_restored_bundle() {
  if [ "${MODE}" = "host" ] && ! { [ -n "${LOGOS_BLOCKCHAIN_NODE_BIN:-}" ] && [ -x "${LOGOS_BLOCKCHAIN_NODE_BIN:-}" ]; }; then
    local tar_node
    tar_node="${RESTORED_BIN_DIR:-${ROOT_DIR}/testing-framework/assets/stack/bin}/logos-blockchain-node"

    [ -x "${tar_node}" ] || common::die \
      "Restored tarball missing host executables; provide a host-compatible binaries tarball."
    run_examples::host_bin_matches_arch "${tar_node}" || common::die \
      "Restored executables do not match host architecture; provide a host-compatible binaries tarball."

    echo "==> Using restored host binaries from tarball"
    LOGOS_BLOCKCHAIN_NODE_BIN="${tar_node}"
    export LOGOS_BLOCKCHAIN_NODE_BIN
  fi
}

run_examples::ensure_circuits() {
  if [ -n "${LOGOS_BLOCKCHAIN_CIRCUITS:-}" ]; then
    if [ -d "${LOGOS_BLOCKCHAIN_CIRCUITS}" ]; then
      return 0
    fi
    common::die "LOGOS_BLOCKCHAIN_CIRCUITS is set to '${LOGOS_BLOCKCHAIN_CIRCUITS}', but the directory does not exist"
  fi

  local default_dir="${HOME}/.logos-blockchain-circuits"
  if [ -d "${default_dir}" ]; then
    LOGOS_BLOCKCHAIN_CIRCUITS="${default_dir}"
    export LOGOS_BLOCKCHAIN_CIRCUITS
    return 0
  fi

  echo "==> Circuits not found; installing to ${default_dir}"
  bash "${ROOT_DIR}/scripts/setup/setup-logos-blockchain-circuits.sh" "${VERSION}" "${default_dir}"
  LOGOS_BLOCKCHAIN_CIRCUITS="${default_dir}"
  export LOGOS_BLOCKCHAIN_CIRCUITS
}

run_examples::run() {
  export LOGOS_BLOCKCHAIN_DEMO_RUN_SECS="${RUN_SECS}"
  export LOGOS_BLOCKCHAIN_DEMO_NODES="${DEMO_NODES}"

  if [ -n "${METRICS_QUERY_URL}" ]; then
    export LOGOS_BLOCKCHAIN_METRICS_QUERY_URL="${METRICS_QUERY_URL}"
  fi
  if [ -n "${METRICS_OTLP_INGEST_URL}" ]; then
    export LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL="${METRICS_OTLP_INGEST_URL}"
  fi

  if [ "${MODE}" = "host" ]; then
    run_examples::ensure_circuits
    # Ensure Groth16 verification keys are embedded when building local node binaries.
    export CARGO_FEATURE_BUILD_VERIFICATION_KEY=1
  fi

  echo "==> Running ${BIN} for ${RUN_SECS}s (mode=${MODE}, image=${IMAGE})"
  cd "${ROOT_DIR}"

  POL_PROOF_DEV_MODE=true \
  TESTNET_PRINT_ENDPOINTS=1 \
  LOGOS_BLOCKCHAIN_TESTNET_IMAGE="${IMAGE}" \
  LOGOS_BLOCKCHAIN_NODE_BIN="${LOGOS_BLOCKCHAIN_NODE_BIN:-}" \
    cargo run -p runner-examples --bin "${BIN}"
}

run_examples::main() {
  run_examples::load_env
  run_examples::parse_args "$@"
  run_examples::select_bin
  run_examples::select_image

  run_examples::prepare_bundles
  echo "==> Using restored binaries bundle"

  SETUP_OUT="$(common::tmpfile nomos-setup-output.XXXXXX)"

  run_examples::maybe_rebuild_image
  run_examples::maybe_restore_host_after_image
  run_examples::validate_restored_bundle
  run_examples::run
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  run_examples::main "$@"
fi
