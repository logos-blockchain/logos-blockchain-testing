#!/usr/bin/env bash
set -euo pipefail

# Thin wrapper; the actual implementation lives in scripts/lib/build-bundle.sh
if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck disable=SC1091
. "${ROOT_DIR}/scripts/lib/build-bundle.sh"

build_bundle::main "$@"
