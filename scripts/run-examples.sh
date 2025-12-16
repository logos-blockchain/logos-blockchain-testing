#!/usr/bin/env bash
set -euo pipefail

# Thin wrapper; the actual implementation lives in scripts/lib/run-examples.sh
# so it can be shared/tested more easily.
if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck disable=SC1091
. "${ROOT_DIR}/scripts/lib/run-examples.sh"

run_examples::main "$@"
