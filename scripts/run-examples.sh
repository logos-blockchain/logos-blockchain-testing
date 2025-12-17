#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

readonly DEFAULT_KZG_DIR_REL="testing-framework/assets/stack/kzgrs_test_params"
readonly DEFAULT_KZG_FILE="kzgrs_test_params"
readonly DEFAULT_KZG_CONTAINER_PATH="/kzgrs_test_params/kzgrs_test_params"
readonly DEFAULT_KZG_IN_IMAGE_PARAMS_PATH="/opt/nomos/kzg-params/kzgrs_test_params"

readonly DEFAULT_LOCAL_IMAGE="logos-blockchain-testing:local"
readonly DEFAULT_PUBLIC_ECR_REGISTRY="public.ecr.aws/r4s5t9y4"
readonly DEFAULT_PUBLIC_ECR_REPO="logos/logos-blockchain"
readonly DEFAULT_PRIVATE_ECR_REPO="logos-blockchain-testing"
readonly DEFAULT_ECR_TAG="test"
readonly DEFAULT_PRIVATE_AWS_REGION="ap-southeast-2"

readonly DEFAULT_PULL_POLICY_LOCAL="IfNotPresent"
readonly DEFAULT_PULL_POLICY_ECR="Always"
readonly DOCKER_DESKTOP_CONTEXT="docker-desktop"

run_examples::usage() {
  cat <<EOF
Usage: scripts/run-examples.sh [options] [compose|host|k8s]

Modes:
  compose   Run examples/src/bin/compose_runner.rs (default)
  host      Run examples/src/bin/local_runner.rs
  k8s       Run examples/src/bin/k8s_runner.rs

Options:
  -t, --run-seconds N     Duration to run the demo (required)
  -v, --validators N      Number of validators (required)
  -e, --executors N       Number of executors (required)
  --bundle PATH           Convenience alias for setting NOMOS_BINARIES_TAR=PATH
  --metrics-query-url URL  (k8s) PromQL base URL the runner process can query (often localhost port-forward)
  --metrics-query-grafana-url URL  (k8s) PromQL base URL reachable from inside the cluster (Grafana datasource)
  --metrics-otlp-ingest-url URL  (k8s) Full OTLP HTTP ingest URL for node metrics export
  --external-prometheus URL  (k8s) Alias for --metrics-query-url
  --external-prometheus-grafana-url URL  (k8s) Alias for --metrics-query-grafana-url
  --external-otlp-metrics-endpoint URL  (k8s) Alias for --metrics-otlp-ingest-url
  --local                 Use a local Docker image tag (default for docker-desktop k8s)
  --ecr                   Use an ECR image reference (default for non-docker-desktop k8s)
  --no-image-build        Skip rebuilding the compose/k8s image (sets NOMOS_SKIP_IMAGE_BUILD=1)

Environment:
  VERSION                          Circuits version (default from versions.env)
  NOMOS_TESTNET_IMAGE              Image reference (overridden by --local/--ecr selection)
  ECR_IMAGE                        Full image reference for --ecr (overrides ECR_REGISTRY/ECR_REPO/TAG)
  ECR_REGISTRY                     Registry hostname for --ecr (default ${DEFAULT_PUBLIC_ECR_REGISTRY})
  ECR_REPO                         Repository path for --ecr (default ${DEFAULT_PUBLIC_ECR_REPO})
  TAG                              Tag for --ecr (default ${DEFAULT_ECR_TAG})
  NOMOS_TESTNET_IMAGE_PULL_POLICY  K8s imagePullPolicy (default ${DEFAULT_PULL_POLICY_LOCAL}; set to ${DEFAULT_PULL_POLICY_ECR} for --ecr)
  NOMOS_BINARIES_TAR               Path to prebuilt binaries/circuits tarball (default .tmp/nomos-binaries-<platform>-<version>.tar.gz)
  NOMOS_SKIP_IMAGE_BUILD           Set to 1 to skip rebuilding the compose/k8s image
  K8S_RUNNER_METRICS_QUERY_URL               PromQL base URL for the runner process
  NOMOS_METRICS_QUERY_URL                    Alias for K8S_RUNNER_METRICS_QUERY_URL
  K8S_RUNNER_METRICS_QUERY_GRAFANA_URL       PromQL base URL for Grafana (cluster-reachable)
  NOMOS_METRICS_QUERY_GRAFANA_URL            Alias for K8S_RUNNER_METRICS_QUERY_GRAFANA_URL
  K8S_RUNNER_METRICS_OTLP_INGEST_URL         Full OTLP HTTP ingest URL for node metrics export
  NOMOS_METRICS_OTLP_INGEST_URL              Alias for K8S_RUNNER_METRICS_OTLP_INGEST_URL

Deprecated env vars (still supported):
  K8S_RUNNER_EXTERNAL_PROMETHEUS_URL, NOMOS_EXTERNAL_PROMETHEUS_URL
  K8S_RUNNER_EXTERNAL_PROMETHEUS_GRAFANA_URL, NOMOS_EXTERNAL_PROMETHEUS_GRAFANA_URL
  K8S_RUNNER_EXTERNAL_OTLP_METRICS_ENDPOINT, NOMOS_EXTERNAL_OTLP_METRICS_ENDPOINT
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

  KZG_DIR_REL="${NOMOS_KZG_DIR_REL:-${DEFAULT_KZG_DIR_REL}}"
  KZG_FILE="${NOMOS_KZG_FILE:-${DEFAULT_KZG_FILE}}"
  KZG_CONTAINER_PATH="${NOMOS_KZG_CONTAINER_PATH:-${DEFAULT_KZG_CONTAINER_PATH}}"
  HOST_KZG_DIR="${ROOT_DIR}/${KZG_DIR_REL}"
  HOST_KZG_FILE="${HOST_KZG_DIR}/${KZG_FILE}"
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
  DEMO_VALIDATORS=""
  DEMO_EXECUTORS=""
  IMAGE_SELECTION_MODE="auto"
  METRICS_QUERY_URL=""
  METRICS_QUERY_GRAFANA_URL=""
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
      -v|--validators)
        DEMO_VALIDATORS="${2:-}"
        shift 2
        ;;
      --validators=*)
        DEMO_VALIDATORS="${1#*=}"
        shift
        ;;
      -e|--executors)
        DEMO_EXECUTORS="${2:-}"
        shift 2
        ;;
      --executors=*)
        DEMO_EXECUTORS="${1#*=}"
        shift
        ;;
      --bundle)
        NOMOS_BINARIES_TAR="${2:-}"
        export NOMOS_BINARIES_TAR
        shift 2
        ;;
      --bundle=*)
        NOMOS_BINARIES_TAR="${1#*=}"
        export NOMOS_BINARIES_TAR
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
      --metrics-query-grafana-url)
        METRICS_QUERY_GRAFANA_URL="${2:-}"
        shift 2
        ;;
      --metrics-query-grafana-url=*)
        METRICS_QUERY_GRAFANA_URL="${1#*=}"
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
      --external-prometheus-grafana-url)
        METRICS_QUERY_GRAFANA_URL="${2:-}"
        shift 2
        ;;
      --external-prometheus-grafana-url=*)
        METRICS_QUERY_GRAFANA_URL="${1#*=}"
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
        if [ "${IMAGE_SELECTION_MODE}" = "ecr" ]; then
          run_examples::fail_with_usage "--local and --ecr are mutually exclusive"
        fi
        IMAGE_SELECTION_MODE="local"
        shift
        ;;
      --ecr)
        if [ "${IMAGE_SELECTION_MODE}" = "local" ]; then
          run_examples::fail_with_usage "--local and --ecr are mutually exclusive"
        fi
        IMAGE_SELECTION_MODE="ecr"
        shift
        ;;
      --no-image-build)
        NOMOS_SKIP_IMAGE_BUILD=1
        export NOMOS_SKIP_IMAGE_BUILD
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

  if [ -n "${NOMOS_BINARIES_TAR:-}" ] && [ ! -f "${NOMOS_BINARIES_TAR}" ]; then
    run_examples::fail_with_usage "NOMOS_BINARIES_TAR is set but missing: ${NOMOS_BINARIES_TAR}"
  fi

  if ! common::is_uint "${RUN_SECS_RAW}" || [ "${RUN_SECS_RAW}" -le 0 ]; then
    run_examples::fail_with_usage "run-seconds must be a positive integer (pass -t/--run-seconds)"
  fi
  RUN_SECS="${RUN_SECS_RAW}"

  if [ -z "${DEMO_VALIDATORS}" ] || [ -z "${DEMO_EXECUTORS}" ]; then
    run_examples::fail_with_usage "validators and executors must be provided via -v/--validators and -e/--executors"
  fi
  if ! common::is_uint "${DEMO_VALIDATORS}" ; then
    run_examples::fail_with_usage "validators must be a non-negative integer (pass -v/--validators)"
  fi
  if ! common::is_uint "${DEMO_EXECUTORS}" ; then
    run_examples::fail_with_usage "executors must be a non-negative integer (pass -e/--executors)"
  fi

  if [ -n "${METRICS_QUERY_URL}" ] && [ "${MODE}" != "k8s" ]; then
    echo "Warning: --metrics-query-url is only used in k8s mode; ignoring." >&2
    METRICS_QUERY_URL=""
  fi
  if [ -n "${METRICS_QUERY_GRAFANA_URL}" ] && [ "${MODE}" != "k8s" ]; then
    echo "Warning: --metrics-query-grafana-url is only used in k8s mode; ignoring." >&2
    METRICS_QUERY_GRAFANA_URL=""
  fi
  if [ -n "${METRICS_OTLP_INGEST_URL}" ] && [ "${MODE}" != "k8s" ]; then
    echo "Warning: --metrics-otlp-ingest-url is only used in k8s mode; ignoring." >&2
    METRICS_OTLP_INGEST_URL=""
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
    IMAGE="${NOMOS_TESTNET_IMAGE:-${DEFAULT_LOCAL_IMAGE}}"
    export NOMOS_TESTNET_IMAGE_PULL_POLICY="${NOMOS_TESTNET_IMAGE_PULL_POLICY:-${DEFAULT_PULL_POLICY_LOCAL}}"
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
    export NOMOS_TESTNET_IMAGE_PULL_POLICY="${NOMOS_TESTNET_IMAGE_PULL_POLICY:-${DEFAULT_PULL_POLICY_ECR}}"
  else
    run_examples::fail_with_usage "Unknown image selection mode: ${selection}"
  fi

  export IMAGE_TAG="${IMAGE}"
  export NOMOS_TESTNET_IMAGE="${IMAGE}"

  if [ "${MODE}" = "k8s" ]; then
    if [ "${selection}" = "ecr" ]; then
      export NOMOS_KZG_MODE="${NOMOS_KZG_MODE:-inImage}"
    else
      export NOMOS_KZG_MODE="${NOMOS_KZG_MODE:-hostPath}"
    fi
  fi
}

run_examples::default_tar_path() {
  if [ -n "${NOMOS_BINARIES_TAR:-}" ]; then
    echo "${NOMOS_BINARIES_TAR}"
    return
  fi
  case "${MODE}" in
    host) echo "${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz" ;;
    compose|k8s)
      if [ "${NOMOS_SKIP_IMAGE_BUILD:-}" = "1" ]; then
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
  [ -z "${NOMOS_NODE_REV:-}" ] && return 0

  local meta tar_rev tar_head
  meta="$(tar -xOzf "${tar_path}" artifacts/nomos-bundle-meta.env 2>/dev/null || true)"
  if [ -z "${meta}" ]; then
    echo "Bundle meta missing in ${tar_path}; treating as stale and rebuilding." >&2
    return 1
  fi
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
  local circuits_src="${src}/circuits"
  local circuits_dst="${HOST_KZG_DIR}"

  RESTORED_BIN_DIR="${src}"
  export RESTORED_BIN_DIR

  if [ ! -f "${src}/nomos-node" ] || [ ! -f "${src}/nomos-executor" ] || [ ! -f "${src}/nomos-cli" ]; then
    echo "Binaries missing in ${tar_path}; provide a prebuilt binaries tarball." >&2
    return 1
  fi

  local copy_bins=1
  if [ "${MODE}" != "host" ] && ! run_examples::host_bin_matches_arch "${src}/nomos-node"; then
    echo "Bundled binaries do not match host arch; skipping copy so containers rebuild from source."
    copy_bins=0
    rm -f "${bin_dst}/nomos-node" "${bin_dst}/nomos-executor" "${bin_dst}/nomos-cli"
  fi
  if [ "${copy_bins}" -eq 1 ]; then
    mkdir -p "${bin_dst}"
    cp "${src}/nomos-node" "${src}/nomos-executor" "${src}/nomos-cli" "${bin_dst}/"
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

run_examples::ensure_binaries_tar() {
  local platform="$1"
  local tar_path="$2"
  echo "==> Building fresh binaries bundle (${platform}) at ${tar_path}"
  "${ROOT_DIR}/scripts/build-bundle.sh" --platform "${platform}" --output "${tar_path}" --rev "${NOMOS_NODE_REV}"
}

run_examples::prepare_bundles() {
  RESTORED_BINARIES=0
  NEED_HOST_RESTORE_AFTER_IMAGE=0

  HOST_TAR="${ROOT_DIR}/.tmp/nomos-binaries-host-${VERSION}.tar.gz"
  LINUX_TAR="${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"

  if [ -n "${NOMOS_NODE_BIN:-}" ] && [ -x "${NOMOS_NODE_BIN}" ] && [ -n "${NOMOS_EXECUTOR_BIN:-}" ] && [ -x "${NOMOS_EXECUTOR_BIN}" ]; then
    echo "==> Using pre-specified host binaries (NOMOS_NODE_BIN/NOMOS_EXECUTOR_BIN); skipping tarball restore"
    return 0
  fi

  # On non-Linux compose/k8s runs, use the Linux bundle for image build, then restore host bundle for the runner.
  if [ "${MODE}" != "host" ] && [ "$(uname -s)" != "Linux" ] && [ "${NOMOS_SKIP_IMAGE_BUILD:-0}" = "0" ] && [ -f "${LINUX_TAR}" ]; then
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
        if [ "${NOMOS_SKIP_IMAGE_BUILD:-0}" = "1" ]; then
          run_examples::ensure_binaries_tar host "${tar_path}"
        else
          run_examples::ensure_binaries_tar linux "${tar_path}"
        fi
        ;;
      *) run_examples::ensure_binaries_tar host "${tar_path}" ;;
    esac

    run_examples::restore_binaries_from_tar "${tar_path}" || common::die \
      "Missing or invalid binaries tarball. Provide it via --bundle/NOMOS_BINARIES_TAR or place it at $(run_examples::default_tar_path)."
  fi
}

run_examples::maybe_rebuild_image() {
  if [ "${MODE}" = "host" ]; then
    return 0
  fi

  if [ "${NOMOS_SKIP_IMAGE_BUILD:-0}" = "1" ]; then
    echo "==> Skipping testnet image rebuild (NOMOS_SKIP_IMAGE_BUILD=1)"
    return 0
  fi

  echo "==> Rebuilding testnet image (${IMAGE})"
  IMAGE_TAG="${IMAGE}" COMPOSE_CIRCUITS_PLATFORM="${COMPOSE_CIRCUITS_PLATFORM:-}" \
    bash "${ROOT_DIR}/scripts/build_test_image.sh"
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
  HOST_BUNDLE_PATH="${HOST_KZG_DIR}"
  KZG_HOST_PATH="${HOST_BUNDLE_PATH}/${KZG_FILE}"

  if [ ! -x "${HOST_BUNDLE_PATH}/zksign/witness_generator" ]; then
    common::die "Missing zksign/witness_generator in restored bundle; ensure the tarball contains host-compatible circuits."
  fi
  if [ ! -f "${KZG_HOST_PATH}" ]; then
    common::die "KZG params missing at ${KZG_HOST_PATH}; ensure the tarball contains circuits."
  fi

  if [ "${MODE}" = "host" ] && ! { [ -n "${NOMOS_NODE_BIN:-}" ] && [ -x "${NOMOS_NODE_BIN:-}" ] && [ -n "${NOMOS_EXECUTOR_BIN:-}" ] && [ -x "${NOMOS_EXECUTOR_BIN:-}" ]; }; then
    local tar_node tar_exec
    tar_node="${RESTORED_BIN_DIR:-${ROOT_DIR}/testing-framework/assets/stack/bin}/nomos-node"
    tar_exec="${RESTORED_BIN_DIR:-${ROOT_DIR}/testing-framework/assets/stack/bin}/nomos-executor"

    [ -x "${tar_node}" ] && [ -x "${tar_exec}" ] || common::die \
      "Restored tarball missing host executables; provide a host-compatible binaries tarball."
    run_examples::host_bin_matches_arch "${tar_node}" && run_examples::host_bin_matches_arch "${tar_exec}" || common::die \
      "Restored executables do not match host architecture; provide a host-compatible binaries tarball."

    echo "==> Using restored host binaries from tarball"
    NOMOS_NODE_BIN="${tar_node}"
    NOMOS_EXECUTOR_BIN="${tar_exec}"
    export NOMOS_NODE_BIN NOMOS_EXECUTOR_BIN
  fi
}

run_examples::kzg_path_for_mode() {
  if [ "${MODE}" = "compose" ] || [ "${MODE}" = "k8s" ]; then
    if [ "${MODE}" = "k8s" ] && [ "${NOMOS_KZG_MODE:-hostPath}" = "inImage" ]; then
      echo "${NOMOS_KZG_IN_IMAGE_PARAMS_PATH:-${DEFAULT_KZG_IN_IMAGE_PARAMS_PATH}}"
    else
      echo "${KZG_CONTAINER_PATH}"
    fi
  else
    echo "${KZG_HOST_PATH}"
  fi
}

run_examples::ensure_compose_circuits_platform_default() {
  if [ "${MODE}" != "compose" ] || [ -n "${COMPOSE_CIRCUITS_PLATFORM:-}" ]; then
    return 0
  fi

  local arch
  arch="$(uname -m)"
  case "${arch}" in
    x86_64) COMPOSE_CIRCUITS_PLATFORM="linux-x86_64" ;;
    arm64|aarch64) COMPOSE_CIRCUITS_PLATFORM="linux-aarch64" ;;
    *) COMPOSE_CIRCUITS_PLATFORM="linux-x86_64" ;;
  esac
  export COMPOSE_CIRCUITS_PLATFORM
}

run_examples::run() {
  local kzg_path
  kzg_path="$(run_examples::kzg_path_for_mode)"

  export NOMOS_DEMO_RUN_SECS="${RUN_SECS}"
  export NOMOS_DEMO_VALIDATORS="${DEMO_VALIDATORS}"
  export NOMOS_DEMO_EXECUTORS="${DEMO_EXECUTORS}"

  if [ "${MODE}" = "k8s" ] && [ -n "${METRICS_QUERY_URL}" ]; then
    export K8S_RUNNER_METRICS_QUERY_URL="${METRICS_QUERY_URL}"
    export NOMOS_METRICS_QUERY_URL="${METRICS_QUERY_URL}"
  fi
  if [ "${MODE}" = "k8s" ] && [ -n "${METRICS_QUERY_GRAFANA_URL}" ]; then
    export K8S_RUNNER_METRICS_QUERY_GRAFANA_URL="${METRICS_QUERY_GRAFANA_URL}"
    export NOMOS_METRICS_QUERY_GRAFANA_URL="${METRICS_QUERY_GRAFANA_URL}"
  fi
  if [ "${MODE}" = "k8s" ] && [ -n "${METRICS_OTLP_INGEST_URL}" ]; then
    export K8S_RUNNER_METRICS_OTLP_INGEST_URL="${METRICS_OTLP_INGEST_URL}"
    export NOMOS_METRICS_OTLP_INGEST_URL="${METRICS_OTLP_INGEST_URL}"
  fi

  echo "==> Running ${BIN} for ${RUN_SECS}s (mode=${MODE}, image=${IMAGE})"
  cd "${ROOT_DIR}"

  POL_PROOF_DEV_MODE=true \
  TESTNET_PRINT_ENDPOINTS=1 \
  NOMOS_TESTNET_IMAGE="${IMAGE}" \
  NOMOS_CIRCUITS="${HOST_BUNDLE_PATH}" \
  NOMOS_KZGRS_PARAMS_PATH="${kzg_path}" \
  NOMOS_NODE_BIN="${NOMOS_NODE_BIN:-}" \
  NOMOS_EXECUTOR_BIN="${NOMOS_EXECUTOR_BIN:-}" \
  COMPOSE_CIRCUITS_PLATFORM="${COMPOSE_CIRCUITS_PLATFORM:-}" \
    cargo run -p runner-examples --bin "${BIN}"
}

run_examples::main() {
  run_examples::load_env
  run_examples::parse_args "$@"
  run_examples::select_bin
  run_examples::select_image

  run_examples::prepare_bundles
  echo "==> Using restored circuits/binaries bundle"

  SETUP_OUT="$(common::tmpfile nomos-setup-output.XXXXXX)"
  cleanup() { rm -f "${SETUP_OUT}" 2>/dev/null || true; }
  trap cleanup EXIT

  run_examples::maybe_rebuild_image
  run_examples::maybe_restore_host_after_image
  run_examples::validate_restored_bundle
  run_examples::ensure_compose_circuits_platform_default
  run_examples::run
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  run_examples::main "$@"
fi
