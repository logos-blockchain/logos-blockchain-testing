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
  scripts/observability/deploy.sh --target compose --action up|down|logs|env
  scripts/observability/deploy.sh --target k8s --action install|uninstall|dashboards|env

Short flags:
  -t, --target   compose|k8s
  -a, --action   (see above)

Examples:
  scripts/observability/deploy.sh -t compose -a up
  eval "$(scripts/observability/deploy.sh -t compose -a env)"

  scripts/observability/deploy.sh -t k8s -a install
  scripts/observability/deploy.sh -t k8s -a dashboards
  scripts/observability/deploy.sh -t k8s -a env
USAGE
}

die_usage() {
  echo "ERROR: $1" >&2
  echo >&2
  usage >&2
  exit 1
}

target=""
action=""

while [ $# -gt 0 ]; do
  case "$1" in
    -t|--target)
      target="${2:-}"; shift 2 ;;
    -a|--action)
      action="${2:-}"; shift 2 ;;
    -h|--help|help)
      usage; exit 0 ;;
    *)
      die_usage "Unknown argument: $1" ;;
  esac
done

[ -n "${target}" ] || die_usage "Missing --target"
[ -n "${action}" ] || die_usage "Missing --action"

case "${target}" in
  compose)
    case "${action}" in
      up|down|logs|env) ;;
      *) die_usage "Invalid compose action: ${action}" ;;
    esac
    ;;
  k8s)
    case "${action}" in
      install|uninstall|dashboards|env) ;;
      *) die_usage "Invalid k8s action: ${action}" ;;
    esac
    ;;
  *)
    die_usage "Invalid --target: ${target} (expected compose|k8s)"
    ;;
esac

exec "${ROOT}/scripts/setup/setup-observability.sh" "${target}" "${action}"

