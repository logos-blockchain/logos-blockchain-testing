#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "${SCRIPT_DIR}/../lib/common.sh"

common::ensure_bash "$@"

ROOT="$(common::repo_root)"

usage() {
  cat <<'USAGE'
Usage:
  scripts/setup/setup-observability.sh compose up|down|logs|env
  scripts/setup/setup-observability.sh k8s install|uninstall|dashboards|env

Compose:
  - Runs Prometheus (+ OTLP receiver) and Grafana via docker compose.
  - Prints LOGOS_BLOCKCHAIN_METRICS_* / LOGOS_BLOCKCHAIN_GRAFANA_URL exports to wire into runs.

Kubernetes:
  - Installs prometheus-community/kube-prometheus-stack into namespace
    "logos-observability" and optionally loads Logos Grafana dashboards.
  - Prints port-forward commands + LOGOS_BLOCKCHAIN_METRICS_* / LOGOS_BLOCKCHAIN_GRAFANA_URL exports.
USAGE
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || common::die "Missing required command: $1"
}

compose_file() {
  echo "${ROOT}/scripts/observability/compose/docker-compose.yml"
}

compose_run() {
  local file
  file="$(compose_file)"
  common::require_file "${file}"
  docker compose -f "${file}" "$@"
}

compose_env() {
  cat <<'EOF'
export LOGOS_BLOCKCHAIN_METRICS_QUERY_URL=http://localhost:9090
export LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL=http://host.docker.internal:9090/api/v1/otlp/v1/metrics
export LOGOS_BLOCKCHAIN_GRAFANA_URL=http://localhost:3000
EOF
}

k8s_namespace() { echo "${LOGOS_OBSERVABILITY_NAMESPACE:-${LOGOS_BLOCKCHAIN_OBSERVABILITY_NAMESPACE:-logos-observability}}"; }
k8s_release() { echo "${LOGOS_OBSERVABILITY_RELEASE:-${LOGOS_BLOCKCHAIN_OBSERVABILITY_RELEASE:-logos-observability}}"; }
k8s_values() { echo "${ROOT}/scripts/observability/k8s/kube-prometheus-stack.values.yaml"; }

k8s_install() {
  require_cmd kubectl
  require_cmd helm

  local ns release values
  ns="$(k8s_namespace)"
  release="$(k8s_release)"
  values="$(k8s_values)"

  common::require_file "${values}"

  kubectl get ns "${ns}" >/dev/null 2>&1 || kubectl create ns "${ns}"

  if ! helm repo list | grep -q '^prometheus-community[[:space:]]'; then
    helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
  fi
  helm repo update prometheus-community

  helm upgrade --install "${release}" prometheus-community/kube-prometheus-stack \
    -n "${ns}" \
    -f "${values}"

  kubectl -n "${ns}" wait --for=condition=Available deploy -l "release=${release}" --timeout=10m || true
  kubectl -n "${ns}" wait --for=condition=Ready pod -l "release=${release}" --timeout=10m || true
}

k8s_uninstall() {
  require_cmd kubectl
  require_cmd helm

  local ns release
  ns="$(k8s_namespace)"
  release="$(k8s_release)"

  helm uninstall "${release}" -n "${ns}" 2>/dev/null || true
  kubectl delete ns "${ns}" --ignore-not-found
}

k8s_apply_dashboards() {
  require_cmd kubectl

  local ns dash_dir
  ns="$(k8s_namespace)"
  dash_dir="${ROOT}/testing-framework/assets/stack/monitoring/grafana/dashboards"

  [ -d "${dash_dir}" ] || common::die "Missing dashboards directory: ${dash_dir}"

  local file base name
  for file in "${dash_dir}"/*.json; do
    base="$(basename "${file}" .json)"
    name="logos-dashboard-${base//[^a-zA-Z0-9-]/-}"
    kubectl -n "${ns}" create configmap "${name}" \
      --from-file="$(basename "${file}")=${file}" \
      --dry-run=client -o yaml | kubectl apply -f -
    kubectl -n "${ns}" label configmap "${name}" grafana_dashboard=1 --overwrite >/dev/null
  done
}

k8s_env() {
  local ns release
  ns="$(k8s_namespace)"
  release="$(k8s_release)"

  cat <<EOF
# Prometheus (runner-side): port-forward then set:
kubectl -n ${ns} port-forward svc/${release}-kube-p-prometheus 9090:9090
export LOGOS_BLOCKCHAIN_METRICS_QUERY_URL=http://localhost:9090

# Grafana (runner-side): port-forward then set:
kubectl -n ${ns} port-forward svc/${release}-grafana 3000:80
export LOGOS_BLOCKCHAIN_GRAFANA_URL=http://localhost:3000

# Prometheus OTLP ingest (node-side inside the cluster):
export LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL=http://${release}-kube-p-prometheus.${ns}:9090/api/v1/otlp/v1/metrics
EOF
}

main() {
  local target="${1:-}"
  local action="${2:-}"

  case "${target}" in
    compose)
      require_cmd docker
      case "${action}" in
        up) compose_run up -d ;;
        down) compose_run down -v ;;
        logs) compose_run logs -f ;;
        env) compose_env ;;
        ""|help|-h|--help) usage ;;
        *) common::die "Unknown compose action: ${action}" ;;
      esac
      ;;
    k8s)
      case "${action}" in
        install) k8s_install ;;
        uninstall) k8s_uninstall ;;
        dashboards) k8s_apply_dashboards ;;
        env) k8s_env ;;
        ""|help|-h|--help) usage ;;
        *) common::die "Unknown k8s action: ${action}" ;;
      esac
      ;;
    ""|help|-h|--help)
      usage
      ;;
    *)
      common::die "Unknown target: ${target}"
      ;;
  esac
}

main "$@"
