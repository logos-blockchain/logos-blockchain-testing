#!/usr/bin/env bash
set -euo pipefail

# Thin wrapper; the actual implementation lives in scripts/lib/build-linux-binaries.sh
if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck disable=SC1091
. "${ROOT_DIR}/scripts/lib/build-linux-binaries.sh"

build_linux_binaries::main "$@"
