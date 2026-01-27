#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../lib/common.sh"

checks::usage() {
  cat <<'USAGE'
Usage: scripts/run/checks.sh [--help]

Runs a best-effort local environment sanity check for the testing framework
(assets, Rust, Docker, Kubernetes).

Notes:
  - This script is informational; it does not modify your system.
  - Many checks are best-effort and may be skipped if tools are missing.
USAGE
}

checks::say() { printf "%s\n" "$*"; }
checks::section() { printf "\n==> %s\n" "$*"; }
checks::have() { command -v "$1" >/dev/null 2>&1; }
checks::warn() { checks::say "WARN: $*"; }
checks::ok() { checks::say "OK: $*"; }

checks::load_env() {
  ROOT_DIR="$(common::repo_root)"
  export ROOT_DIR

  if [ -f "${ROOT_DIR}/versions.env" ]; then
    # shellcheck disable=SC1091
    . "${ROOT_DIR}/versions.env"
  fi
  if [ -f "${ROOT_DIR}/paths.env" ]; then
    # shellcheck disable=SC1091
    . "${ROOT_DIR}/paths.env"
  fi
}

checks::print_workspace() {
  checks::section "Workspace"
  checks::say "root: ${ROOT_DIR}"
  if [ -f "${ROOT_DIR}/versions.env" ]; then
    checks::ok "versions.env present"
    checks::say "VERSION=${VERSION:-<unset>}"
    checks::say "LOGOS_BLOCKCHAIN_NODE_REV=${LOGOS_BLOCKCHAIN_NODE_REV:-<unset>}"
    if [ -n "${LOGOS_BLOCKCHAIN_NODE_PATH:-}" ]; then
      checks::say "LOGOS_BLOCKCHAIN_NODE_PATH=${LOGOS_BLOCKCHAIN_NODE_PATH}"
    fi
  else
    checks::warn "versions.env missing (scripts depend on it)"
  fi

  if [ -f "${ROOT_DIR}/paths.env" ]; then
    checks::ok "paths.env present"
  fi
}

checks::print_disk_space() {
  checks::section "Disk Space"
  if checks::have df; then
    df -h "${ROOT_DIR}" | sed -n '1,2p'
  fi

  local tmp_dir="${ROOT_DIR}/.tmp"
  if [ -d "${tmp_dir}" ]; then
    if checks::have du; then
      checks::say ".tmp size: $(du -sh "${tmp_dir}" 2>/dev/null | awk '{print $1}')"
    fi
  else
    checks::say ".tmp: <absent>"
  fi

  if [ -d "${ROOT_DIR}/target" ] && checks::have du; then
    checks::say "target size: $(du -sh "${ROOT_DIR}/target" 2>/dev/null | awk '{print $1}')"
  fi
}


checks::print_rust_toolchain() {
  checks::section "Rust Toolchain"
  if checks::have rustup; then
    checks::ok "rustup: $(rustup --version | head -n1)"
    if [ -f "${ROOT_DIR}/rust-toolchain.toml" ]; then
      local channel
      channel="$(awk -F '\"' '/^[[:space:]]*channel[[:space:]]*=/{print $2; exit}' "${ROOT_DIR}/rust-toolchain.toml" 2>/dev/null || true)"
      checks::say "rust-toolchain.toml channel: ${channel:-<unknown>}"
    fi
  elif checks::have rustc; then
    checks::ok "rustc: $(rustc --version)"
  else
    checks::warn "rust toolchain not found (rustup/rustc missing)"
  fi
}

checks::print_docker() {
  checks::section "Docker (compose/k8s image + linux bundle builds)"

  local default_local_image="logos-blockchain-testing:local"
  local default_bundle_platform_amd64="linux/amd64"
  local default_bundle_platform_arm64="linux/arm64"

  if ! checks::have docker; then
    checks::warn "docker not found (compose/k8s unavailable; linux bundle build on macOS requires docker)"
    return 0
  fi

  checks::ok "docker client: $(docker version --format '{{.Client.Version}}' 2>/dev/null || docker --version)"
  local server_arch
  server_arch="$(docker version --format '{{.Server.Os}}/{{.Server.Arch}}' 2>/dev/null || true)"
  if [ -n "${server_arch}" ]; then
    checks::say "docker engine: ${server_arch}"
  else
    checks::warn "could not query docker engine arch (is Docker running?)"
  fi

  local bundle_platform="${LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM:-${LOGOS_BLOCKCHAIN_BIN_PLATFORM:-}}"
  if [ -z "${bundle_platform}" ]; then
    checks::say "LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM=<auto>"
    if [[ "${server_arch}" == *"linux/arm64"* ]]; then
      checks::say "bundle docker platform (auto): ${default_bundle_platform_arm64}"
    else
      checks::say "bundle docker platform (auto): ${default_bundle_platform_amd64}"
    fi
    bundle_platform="auto"
  else
    checks::say "LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM=${bundle_platform}"
  fi

  if [[ "${server_arch}" == *"linux/arm64"* ]] && [ "${bundle_platform}" = "${default_bundle_platform_amd64}" ]; then
    checks::warn "Docker engine is linux/arm64 but bundle platform is ${default_bundle_platform_amd64} (emulation). If builds are slow/flaky, set: LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM=${default_bundle_platform_arm64}"
  fi

  local image="${LOGOS_BLOCKCHAIN_TESTNET_IMAGE:-${default_local_image}}"
  checks::say "LOGOS_BLOCKCHAIN_TESTNET_IMAGE=${image}"
  if docker image inspect "${image}" >/dev/null 2>&1; then
    checks::ok "testnet image present locally"
  else
    checks::warn "testnet image not present locally (compose/k8s runs will rebuild or fail if LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD=1)"
  fi
}

checks::print_docker_compose() {
  checks::section "Docker Compose"
  if checks::have docker; then
    if docker compose version >/dev/null 2>&1; then
      checks::ok "docker compose available"
    else
      checks::warn "docker compose not available"
    fi
  fi
}

checks::print_kubernetes() {
  checks::section "Kubernetes (k8s runner)"

  if checks::have kubectl; then
    checks::ok "kubectl: $(kubectl version --client=true --short 2>/dev/null || true)"
    KUBE_CONTEXT="$(kubectl config current-context 2>/dev/null || true)"
    if [ -n "${KUBE_CONTEXT}" ]; then
      checks::say "current-context: ${KUBE_CONTEXT}"
    fi
    if kubectl cluster-info >/dev/null 2>&1; then
      checks::ok "cluster reachable"
      kubectl get nodes -o wide 2>/dev/null | sed -n '1,3p' || true
    else
      checks::warn "cluster not reachable (k8s runner will skip with ClientInit error)"
    fi
  else
    checks::warn "kubectl not found (k8s runner unavailable)"
    KUBE_CONTEXT=""
  fi

  if checks::have helm; then
    checks::ok "helm: $(helm version --short 2>/dev/null || true)"
  else
    checks::warn "helm not found (k8s runner uses helm)"
  fi
}

checks::print_k8s_image_visibility() {
  checks::section "K8s Image Visibility"

  local default_local_image="logos-blockchain-testing:local"
  local image="${LOGOS_BLOCKCHAIN_TESTNET_IMAGE:-${default_local_image}}"

  if [ -z "${KUBE_CONTEXT:-}" ]; then
    return 0
  fi

  case "${KUBE_CONTEXT}" in
    docker-desktop)
      checks::ok "docker-desktop context shares local Docker images"
      ;;
    kind-*)
      if [[ "${image}" == *":local" ]]; then
        checks::warn "kind cluster won't see local Docker images by default"
        checks::say "Suggested: kind load docker-image ${image}"
      fi
      ;;
    minikube)
      if [[ "${image}" == *":local" ]]; then
        checks::warn "minikube may not see local Docker images by default"
        checks::say "Suggested: minikube image load ${image}"
      fi
      ;;
    *)
      if [[ "${image}" == *":local" ]]; then
        checks::warn "current context is ${KUBE_CONTEXT}; a :local image tag may not be reachable by cluster nodes"
        checks::say "Suggested: push to a registry and set LOGOS_BLOCKCHAIN_TESTNET_IMAGE, or load into the cluster if supported"
      fi
      ;;
  esac
}

checks::print_docker_desktop_kubernetes_health() {
  checks::section "Docker Desktop Kubernetes Health (best-effort)"

  if ! checks::have kubectl; then
    return 0
  fi
  if [ "${KUBE_CONTEXT:-}" != "docker-desktop" ]; then
    return 0
  fi

  local kube_system_namespace="kube-system"
  local storage_provisioner_pod="storage-provisioner"

  if ! kubectl -n "${kube_system_namespace}" get pod "${storage_provisioner_pod}" >/dev/null 2>&1; then
    checks::warn "${storage_provisioner_pod} pod not found"
    return 0
  fi

  local phase reason
  phase="$(kubectl -n "${kube_system_namespace}" get pod "${storage_provisioner_pod}" -o jsonpath='{.status.phase}' 2>/dev/null || true)"
  reason="$(kubectl -n "${kube_system_namespace}" get pod "${storage_provisioner_pod}" -o jsonpath='{.status.containerStatuses[0].state.waiting.reason}' 2>/dev/null || true)"
  if [ "${phase}" = "Running" ] || [ "${phase}" = "Succeeded" ]; then
    checks::ok "${storage_provisioner_pod}: ${phase}"
  else
    checks::warn "${storage_provisioner_pod}: ${phase:-<unknown>} ${reason}"
  fi
}

checks::print_debug_flags() {
  checks::section "Runner Debug Flags (optional)"
  checks::say "SLOW_TEST_ENV=${SLOW_TEST_ENV:-<unset>}  (if true: doubles readiness timeouts)"
  checks::say "LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD=${LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD:-<unset>}  (compose/k8s)"
  checks::say "COMPOSE_RUNNER_PRESERVE=${COMPOSE_RUNNER_PRESERVE:-<unset>}  (compose)"
  checks::say "K8S_RUNNER_PRESERVE=${K8S_RUNNER_PRESERVE:-<unset>}  (k8s)"
  checks::say "K8S_RUNNER_DEBUG=${K8S_RUNNER_DEBUG:-<unset>}  (k8s helm debug)"
  checks::say "COMPOSE_RUNNER_HOST=${COMPOSE_RUNNER_HOST:-<unset>}  (compose readiness host override)"
  checks::say "K8S_RUNNER_NODE_HOST=${K8S_RUNNER_NODE_HOST:-<unset>}  (k8s NodePort host override)"
  checks::say "K8S_RUNNER_NAMESPACE=${K8S_RUNNER_NAMESPACE:-<unset>}  (k8s fixed namespace)"
}

checks::main() {
  case "${1:-}" in
    -h|--help) checks::usage; exit 0 ;;
  esac

  checks::load_env
  checks::print_workspace
  checks::print_disk_space
  checks::print_rust_toolchain
  checks::print_docker
  checks::print_docker_compose
  checks::print_kubernetes
  checks::print_k8s_image_visibility
  checks::print_docker_desktop_kubernetes_health
  checks::print_debug_flags

  checks::section "Done"
  checks::say "If something looks off, start with: scripts/run/run-examples.sh <mode> -t 60 -n 1"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  checks::main "$@"
fi
