#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

clean::usage() {
  cat <<'USAGE'
Usage: scripts/clean.sh [options]

Removes local build artifacts that commonly cause disk pressure and flaky Docker builds.

Options:
  --tmp           Remove .tmp (default)
  --target        Remove target (default)
  --docker        Prune Docker builder cache (docker builder prune -f)
  --docker-system Prune Docker system objects (requires --dangerous)
  --volumes       With --docker-system, also prune volumes
  --dangerous     Required for --docker-system (destructive)
  --all           Equivalent to --tmp --target --docker
  -h, --help      Show this help
USAGE
}

clean::have() { command -v "$1" >/dev/null 2>&1; }
clean::warn() { printf "WARN: %s\n" "$*" >&2; }
clean::die_usage() { printf "ERROR: %s\n" "$*" >&2; clean::usage >&2; exit 2; }

clean::parse_args() {
  DO_TMP=0
  DO_TARGET=0
  DO_DOCKER=0
  DO_DOCKER_SYSTEM=0
  DO_VOLUMES=0
  DANGEROUS=0

  if [ "$#" -eq 0 ]; then
    DO_TMP=1
    DO_TARGET=1
  fi

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --tmp) DO_TMP=1; shift ;;
      --target) DO_TARGET=1; shift ;;
      --docker) DO_DOCKER=1; shift ;;
      --docker-system) DO_DOCKER_SYSTEM=1; shift ;;
      --volumes) DO_VOLUMES=1; shift ;;
      --dangerous) DANGEROUS=1; shift ;;
      --all) DO_TMP=1; DO_TARGET=1; DO_DOCKER=1; shift ;;
      -h|--help) clean::usage; exit 0 ;;
      *) clean::die_usage "Unknown argument: $1" ;;
    esac
  done
}

clean::rm_path() {
  local path="$1"
  if [ -e "${path}" ]; then
    echo "==> Removing ${path}"
    rm -rf "${path}"
  else
    echo "==> Skipping missing ${path}"
  fi
}

clean::docker_prune_builder() {
  if clean::have docker; then
    echo "==> Pruning Docker builder cache"
    docker builder prune -f >/dev/null
    echo "==> Docker builder cache pruned"
  else
    clean::warn "docker not found; skipping Docker prune"
  fi
}

clean::docker_prune_system() {
  if [ "${DANGEROUS}" -ne 1 ]; then
    clean::die_usage "--docker-system requires --dangerous"
  fi
  if clean::have docker; then
    echo "==> Pruning Docker system objects"
    if [ "${DO_VOLUMES}" -eq 1 ]; then
      docker system prune -af --volumes >/dev/null
    else
      docker system prune -af >/dev/null
    fi
    echo "==> Docker system prune complete"
  else
    clean::warn "docker not found; skipping Docker system prune"
  fi
}

clean::main() {
  clean::parse_args "$@"

  ROOT_DIR="$(common::repo_root)"
  echo "Workspace: ${ROOT_DIR}"

  if [ "${DO_TMP}" -eq 1 ]; then
    clean::rm_path "${ROOT_DIR}/.tmp"
  fi
  if [ "${DO_TARGET}" -eq 1 ]; then
    clean::rm_path "${ROOT_DIR}/target"
  fi
  if [ "${DO_DOCKER}" -eq 1 ]; then
    clean::docker_prune_builder
  fi
  if [ "${DO_DOCKER_SYSTEM}" -eq 1 ]; then
    clean::docker_prune_system
  fi

  echo "Done."
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  clean::main "$@"
fi
