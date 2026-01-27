#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../lib/common.sh"

matrix::usage() {
  cat <<'USAGE'
Usage: scripts/run/run-test-matrix.sh [options]

Runs a small matrix of runner examples (host/compose/k8s) with and without
image rebuilds (where it makes sense), after cleaning and rebuilding bundles.

Options:
  -t, --run-seconds N     Demo duration for each run (default: 120)
  -n, --nodes N           Nodes (default: 1)
  --modes LIST            Comma-separated: host,compose,k8s (default: host,compose,k8s)
  --no-clean              Skip scripts/ops/clean.sh step
  --no-bundles            Skip scripts/build/build-bundle.sh (uses existing .tmp tarballs)
  --no-image-build        Skip image-build variants (compose/k8s); only run the --no-image-build cases
  --allow-nonzero-progress  Treat expectation failures as success if logs show non-zero progress (for faster local iteration)
  --force-k8s-image-build Allow the k8s "rebuild image" run even on non-docker-desktop clusters
  --metrics-query-url URL       Forwarded to scripts/run/run-examples.sh (optional)
  --metrics-otlp-ingest-url URL Forwarded to scripts/run/run-examples.sh (optional)
  -h, --help              Show this help

Notes:
  - For k8s on non-docker-desktop clusters, the matrix defaults to only the
    --no-image-build variant, since a local docker build is not visible to the cluster.
  - --allow-nonzero-progress is intentionally lax and should not be used in CI.
USAGE
}

matrix::die() {
  echo "ERROR: $*" >&2
  exit 2
}

matrix::have() { command -v "$1" >/dev/null 2>&1; }

matrix::parse_args() {
  RUN_SECS=120
  NODES=1
  MODES_RAW="host,compose,k8s"
  DO_CLEAN=1
  DO_BUNDLES=1
  SKIP_IMAGE_BUILD_VARIANTS=0
  ALLOW_NONZERO_PROGRESS=0
  FORCE_K8S_IMAGE_BUILD=0
  METRICS_QUERY_URL=""
  METRICS_OTLP_INGEST_URL=""

  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h|--help) matrix::usage; exit 0 ;;
      -t|--run-seconds) RUN_SECS="${2:-}"; shift 2 ;;
      --run-seconds=*) RUN_SECS="${1#*=}"; shift ;;
      -n|--nodes) NODES="${2:-}"; shift 2 ;;
      --nodes=*) NODES="${1#*=}"; shift ;;
      --modes) MODES_RAW="${2:-}"; shift 2 ;;
      --modes=*) MODES_RAW="${1#*=}"; shift ;;
      --no-clean) DO_CLEAN=0; shift ;;
      --no-bundles) DO_BUNDLES=0; shift ;;
      --no-image-build) SKIP_IMAGE_BUILD_VARIANTS=1; shift ;;
      --allow-nonzero-progress) ALLOW_NONZERO_PROGRESS=1; shift ;;
      --force-k8s-image-build) FORCE_K8S_IMAGE_BUILD=1; shift ;;
      --metrics-query-url) METRICS_QUERY_URL="${2:-}"; shift 2 ;;
      --metrics-query-url=*) METRICS_QUERY_URL="${1#*=}"; shift ;;
      --metrics-otlp-ingest-url) METRICS_OTLP_INGEST_URL="${2:-}"; shift 2 ;;
      --metrics-otlp-ingest-url=*) METRICS_OTLP_INGEST_URL="${1#*=}"; shift ;;
      *) matrix::die "Unknown argument: $1" ;;
    esac
  done

  common::is_uint "${RUN_SECS}" || matrix::die "--run-seconds must be an integer"
  [ "${RUN_SECS}" -gt 0 ] || matrix::die "--run-seconds must be > 0"
  common::is_uint "${NODES}" || matrix::die "--nodes must be an integer"
}

matrix::split_modes() {
  MODES=()
  local token
  IFS=',' read -r -a MODES <<< "${MODES_RAW}"
  for token in "${MODES[@]}"; do
    case "${token}" in
      host|compose|k8s) ;;
      "") ;;
      *) matrix::die "Unknown mode in --modes: ${token} (expected host,compose,k8s)" ;;
    esac
  done
}

matrix::forwarded_args() {
  local -a args=()
  if [ -n "${METRICS_QUERY_URL}" ]; then
    args+=(--metrics-query-url "${METRICS_QUERY_URL}")
  fi
  if [ -n "${METRICS_OTLP_INGEST_URL}" ]; then
    args+=(--metrics-otlp-ingest-url "${METRICS_OTLP_INGEST_URL}")
  fi
  printf '%s\0' "${args[@]}"
}

matrix::log_has_nonzero_progress() {
  local log="$1"
  # Heuristic: "some progress happened" means we observed some non-zero metric
  # from the runner/expectations. Keep this intentionally lax; it's a local
  # iteration escape hatch, not a CI signal.
  #
  # Use portable awk (no 3-arg match()) so this works on macOS / busybox.
  awk '
    function record_num(re) {
      if (match($0, re)) {
        s = substr($0, RSTART, RLENGTH)
        gsub(/[^0-9]/, "", s)
        if (s + 0 > 0) { ok = 1 }
      }
    }
    {
      record_num(/height[^0-9]*[0-9]+/)
      record_num(/observed_blocks[^0-9]*[0-9]+/)
      record_num(/observed_total_blobs[^0-9]*[0-9]+/)
      record_num(/channels_with_blobs[^0-9]*[0-9]+/)
      record_num(/inscriptions_observed[^0-9]*[0-9]+/)
      record_num(/observed[^0-9]*[0-9]+\/[0-9]+/)
    }
    END { exit ok ? 0 : 1 }
  ' "${log}"
}

matrix::run_case() {
  local name="$1"
  shift

  local log="${LOG_DIR}/${name}.log"
  mkdir -p "$(dirname "${log}")"
  echo "==> [${name}] $(date -u +'%Y-%m-%dT%H:%M:%SZ')"
  echo "==> [${name}] cmd: $*"

  local start end status
  start="$(date +%s)"
  set +e
  "$@" 2>&1 | tee "${log}"
  status="${PIPESTATUS[0]}"
  set -e

  if [ "${status}" -ne 0 ] && [ "${ALLOW_NONZERO_PROGRESS}" -eq 1 ]; then
    if matrix::log_has_nonzero_progress "${log}"; then
      echo "==> [${name}] Soft-passing due to non-zero progress (--allow-nonzero-progress)"
      status=0
    fi
  fi
  end="$(date +%s)"

  CASE_NAMES+=("${name}")
  CASE_CODES+=("${status}")
  CASE_SECS+=("$((end - start))")

  if [ "${status}" -ne 0 ]; then
    echo "==> [${name}] FAILED (exit=${status}, secs=$((end - start)))"
  else
    echo "==> [${name}] OK (secs=$((end - start)))"
  fi
}

matrix::k8s_context() {
  if ! matrix::have kubectl; then
    echo ""
    return 0
  fi
  kubectl config current-context 2>/dev/null || true
}

matrix::main() {
  ROOT_DIR="$(common::repo_root)"
  export ROOT_DIR
  export RUST_LOG="${RUST_LOG:-info}"

  matrix::parse_args "$@"
  matrix::split_modes

  local ts
  ts="$(date +%Y%m%d-%H%M%S)"
  LOG_DIR="${ROOT_DIR}/.tmp/test-matrix/${ts}"
  mkdir -p "${LOG_DIR}"

  echo "Workspace: ${ROOT_DIR}"
  echo "Logs: ${LOG_DIR}"

  if [ "${DO_CLEAN}" -eq 1 ]; then
    echo "==> Cleaning workspace artifacts"
    "${ROOT_DIR}/scripts/ops/clean.sh" --tmp --target --docker
  fi

  if [ "${DO_BUNDLES}" -eq 1 ]; then
    echo "==> Building bundles (host + linux)"
    "${ROOT_DIR}/scripts/build/build-bundle.sh" --platform host
    "${ROOT_DIR}/scripts/build/build-bundle.sh" --platform linux
  fi

  CASE_NAMES=()
  CASE_CODES=()
  CASE_SECS=()

  local -a forward
  IFS=$'\0' read -r -d '' -a forward < <(matrix::forwarded_args; printf '\0')

  local mode
  for mode in "${MODES[@]}"; do
    case "${mode}" in
      host)
        matrix::run_case "host" \
          "${ROOT_DIR}/scripts/run/run-examples.sh" \
            -t "${RUN_SECS}" -n "${NODES}" \
            "${forward[@]}" \
            host
        ;;
      compose)
        if [ "${SKIP_IMAGE_BUILD_VARIANTS}" -eq 0 ]; then
          matrix::run_case "compose.image_build" \
            "${ROOT_DIR}/scripts/run/run-examples.sh" \
              -t "${RUN_SECS}" -n "${NODES}" \
              "${forward[@]}" \
              compose
        else
          echo "==> [compose] Skipping image-build variant (--no-image-build)"
        fi

        matrix::run_case "compose.skip_image_build" \
          "${ROOT_DIR}/scripts/run/run-examples.sh" \
            --no-image-build \
            -t "${RUN_SECS}" -n "${NODES}" \
            "${forward[@]}" \
            compose
        ;;
      k8s)
        local ctx
        ctx="$(matrix::k8s_context)"
        if [ -z "${ctx}" ]; then
          echo "==> [k8s] Skipping (kubectl missing or no current context)"
          continue
        fi

        if [ "${SKIP_IMAGE_BUILD_VARIANTS}" -eq 0 ]; then
          if [ "${ctx}" = "docker-desktop" ] || [ "${FORCE_K8S_IMAGE_BUILD}" -eq 1 ]; then
            # On non-docker-desktop clusters, run-examples.sh defaults to skipping local image builds
            # since the cluster can't see them. Honor the matrix "force" option by overriding.
            if [ "${ctx}" != "docker-desktop" ] && [ "${FORCE_K8S_IMAGE_BUILD}" -eq 1 ]; then
              export LOGOS_BLOCKCHAIN_FORCE_IMAGE_BUILD=1
            fi
            matrix::run_case "k8s.image_build" \
              "${ROOT_DIR}/scripts/run/run-examples.sh" \
                -t "${RUN_SECS}" -n "${NODES}" \
                "${forward[@]}" \
                k8s
            unset LOGOS_BLOCKCHAIN_FORCE_IMAGE_BUILD || true
          else
            echo "==> [k8s] Detected context '${ctx}'; skipping image-build variant (use --force-k8s-image-build to override)"
          fi
        else
          echo "==> [k8s] Skipping image-build variant (--no-image-build)"
        fi

        matrix::run_case "k8s.skip_image_build" \
          "${ROOT_DIR}/scripts/run/run-examples.sh" \
            --no-image-build \
            -t "${RUN_SECS}" -n "${NODES}" \
            "${forward[@]}" \
            k8s
        ;;
      "")
        ;;
      *)
        matrix::die "Unhandled mode: ${mode}"
        ;;
    esac
  done

  echo "==> Summary"
  local i failed=0
  for i in "${!CASE_NAMES[@]}"; do
    printf "%-28s exit=%-3s secs=%s log=%s\n" \
      "${CASE_NAMES[$i]}" \
      "${CASE_CODES[$i]}" \
      "${CASE_SECS[$i]}" \
      "${LOG_DIR}/${CASE_NAMES[$i]}.log"
    if [ "${CASE_CODES[$i]}" -ne 0 ]; then
      failed=1
    fi
  done

  if [ "${failed}" -ne 0 ]; then
    echo "==> Matrix FAILED (see logs under ${LOG_DIR})" >&2
    exit 1
  fi

  echo "==> Matrix OK"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  matrix::main "$@"
fi
