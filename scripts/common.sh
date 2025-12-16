#!/usr/bin/env bash
set -euo pipefail

# Shared helpers for `scripts/*.sh`.

common::ensure_bash() {
  if [ -z "${BASH_VERSION:-}" ]; then
    exec bash "$0" "$@"
  fi
}

common::repo_root() {
  local caller_source="${BASH_SOURCE[1]:-${BASH_SOURCE[0]}}"
  local dir
  dir="$(cd "$(dirname "${caller_source}")" && pwd)"
  while true; do
    if [ -f "${dir}/versions.env" ]; then
      echo "${dir}"
      return 0
    fi
    if [ "${dir}" = "/" ]; then
      common::die "Could not locate repo root (versions.env) from ${caller_source}"
    fi
    dir="$(cd "${dir}/.." && pwd)"
  done
}

common::die() {
  echo "ERROR: $1" >&2
  exit 1
}

common::is_uint() {
  [[ "${1:-}" =~ ^[0-9]+$ ]]
}

common::require_file() {
  local path="$1"
  [ -f "${path}" ] || common::die "Missing required file: ${path}"
}

common::maybe_source() {
  local path="$1"
  if [ -f "${path}" ]; then
    # shellcheck disable=SC1090
    . "${path}"
  fi
}

common::tmpfile() {
  # macOS and GNU mktemp have slightly different flags; -t works on macOS.
  mktemp -t "${1:-tmp.XXXXXX}"
}

common::tmpdir() {
  # macOS and GNU mktemp have slightly different flags; -t works on macOS.
  mktemp -d -t "${1:-tmpdir.XXXXXX}"
}
