#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../lib/common.sh"

readonly DEFAULT_CIRCUITS_VERSION="v0.3.1"
readonly DEFAULT_LINUX_PLATFORM="linux-x86_64"

readonly DEFAULT_KZG_DIR_REL="testing-framework/assets/stack/kzgrs_test_params"
readonly DEFAULT_KZG_FILE="kzgrs_test_params"
readonly DEFAULT_KZG_PARAMS_RELPATH="tests/kzgrs/kzgrs_test_params"
readonly RAW_GITHUB_BASE_URL="https://raw.githubusercontent.com"

setup_circuits_stack::usage() {
  cat <<'EOF'
Usage: scripts/setup/setup-circuits-stack.sh [VERSION]

Prepares circuits for both the Docker image (Linux/x86_64) and the host (for
witness generators).

Env overrides:
  STACK_DIR   Where to place the Linux bundle (default: testing-framework/assets/stack/kzgrs_test_params)
  HOST_DIR    Where to place the host bundle (default: .tmp/nomos-circuits-host)
  LINUX_STAGE_DIR  Optional staging dir for the Linux bundle (default: .tmp/nomos-circuits-linux)
  NOMOS_CIRCUITS_PLATFORM            Force host platform (e.g., macos-aarch64)
  NOMOS_CIRCUITS_REBUILD_RAPIDSNARK  Set to 1 to force rebuild (host bundle only)
EOF
}

setup_circuits_stack::fail_with_usage() {
  echo "$1" >&2
  setup_circuits_stack::usage
  exit 1
}

setup_circuits_stack::realpath_py() {
  python3 - "$1" <<'PY'
import os, sys
print(os.path.realpath(sys.argv[1]))
PY
}

setup_circuits_stack::detect_platform() {
  local os arch
  case "$(uname -s)" in
    Linux*) os="linux" ;;
    Darwin*) os="macos" ;;
    MINGW*|MSYS*|CYGWIN*) os="windows" ;;
    *) common::die "Unsupported OS: $(uname -s)" ;;
  esac

  case "$(uname -m)" in
    x86_64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) common::die "Unsupported arch: $(uname -m)" ;;
  esac

  echo "${os}-${arch}"
}

setup_circuits_stack::fetch_bundle() {
  local platform="$1"
  local dest="$2"
  local rebuild="${3:-0}"

  # Install into a temporary directory first and only replace `${dest}` once we
  # have a complete bundle. This avoids deleting a working install if GitHub
  # returns transient errors (e.g. 502/504).
  local temp_dest
  temp_dest="$(mktemp -d)"

  if ! NOMOS_CIRCUITS_PLATFORM="${platform}" \
    NOMOS_CIRCUITS_REBUILD_RAPIDSNARK="${rebuild}" \
    "${ROOT_DIR}/scripts/setup/setup-nomos-circuits.sh" "${VERSION}" "${temp_dest}"; then
    rm -rf "${temp_dest}"
    return 1
  fi

  rm -rf "${dest}"
  mkdir -p "$(dirname "${dest}")"
  mv "${temp_dest}" "${dest}"
}

setup_circuits_stack::fetch_kzg_params() {
  local dest_dir="$1"
  local dest_file="${dest_dir}/${KZG_FILE}"
  local url="${RAW_GITHUB_BASE_URL}/logos-co/nomos-node/${NOMOS_NODE_REV}/${DEFAULT_KZG_PARAMS_RELPATH}"

  echo "Fetching KZG parameters from ${url}"
  curl -fsSL "${url}" -o "${dest_file}"
}

setup_circuits_stack::load_env() {
  ROOT_DIR="$(common::repo_root)"
  export ROOT_DIR

  common::require_file "${ROOT_DIR}/versions.env"
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/versions.env"
  common::maybe_source "${ROOT_DIR}/paths.env"

  KZG_DIR_REL="${NOMOS_KZG_DIR_REL:-${DEFAULT_KZG_DIR_REL}}"
  KZG_FILE="${NOMOS_KZG_FILE:-${DEFAULT_KZG_FILE}}"
  HOST_DIR_REL_DEFAULT="${NOMOS_CIRCUITS_HOST_DIR_REL:-.tmp/nomos-circuits-host}"
  LINUX_DIR_REL_DEFAULT="${NOMOS_CIRCUITS_LINUX_DIR_REL:-.tmp/nomos-circuits-linux}"

  VERSION="${VERSION:-${DEFAULT_CIRCUITS_VERSION}}"
  STACK_DIR="${STACK_DIR:-${ROOT_DIR}/${KZG_DIR_REL}}"
  HOST_DIR="${HOST_DIR:-${ROOT_DIR}/${HOST_DIR_REL_DEFAULT}}"
  LINUX_STAGE_DIR="${LINUX_STAGE_DIR:-${ROOT_DIR}/${LINUX_DIR_REL_DEFAULT}}"

  NOMOS_NODE_REV="${NOMOS_NODE_REV:?Missing NOMOS_NODE_REV in versions.env or env}"

  # Force non-interactive installs so repeated runs do not prompt.
  export NOMOS_CIRCUITS_NONINTERACTIVE=1
}

setup_circuits_stack::main() {
  if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
    setup_circuits_stack::usage
    exit 0
  fi

  setup_circuits_stack::load_env
  if [ -n "${1:-}" ]; then
    VERSION="$1"
  fi

  echo "Preparing circuits (version ${VERSION})"
  echo "Workspace: ${ROOT_DIR}"

  local linux_platform="${DEFAULT_LINUX_PLATFORM}"

  echo "Installing Linux bundle for Docker image into ${STACK_DIR}"
  local stage_real stack_real
  stage_real="$(setup_circuits_stack::realpath_py "${LINUX_STAGE_DIR}")"
  stack_real="$(setup_circuits_stack::realpath_py "${STACK_DIR}")"

  if [ "${stage_real}" = "${stack_real}" ]; then
    rm -rf "${STACK_DIR}"
    setup_circuits_stack::fetch_bundle "${linux_platform}" "${STACK_DIR}" 0
    setup_circuits_stack::fetch_kzg_params "${STACK_DIR}"
  else
    rm -rf "${LINUX_STAGE_DIR}"
    mkdir -p "${LINUX_STAGE_DIR}"
    setup_circuits_stack::fetch_bundle "${linux_platform}" "${LINUX_STAGE_DIR}" 0
    rm -rf "${STACK_DIR}"
    mkdir -p "${STACK_DIR}"
    cp -R "${LINUX_STAGE_DIR}/." "${STACK_DIR}/"
    setup_circuits_stack::fetch_kzg_params "${STACK_DIR}"
  fi
  echo "Linux bundle ready at ${STACK_DIR}"

  local host_platform
  host_platform="${NOMOS_CIRCUITS_PLATFORM:-$(setup_circuits_stack::detect_platform)}"
  if [[ "${host_platform}" == "${linux_platform}" ]]; then
    echo "Host platform ${host_platform} matches Linux bundle; host can reuse ${STACK_DIR}"
    echo "Export if you want to be explicit:"
    echo "  export NOMOS_CIRCUITS=\"${STACK_DIR}\""
  else
    echo "Host platform detected: ${host_platform}; installing host-native bundle into ${HOST_DIR}"
    setup_circuits_stack::fetch_bundle "${host_platform}" "${HOST_DIR}" "${NOMOS_CIRCUITS_REBUILD_RAPIDSNARK:-0}"
    setup_circuits_stack::fetch_kzg_params "${HOST_DIR}"
    echo "Host bundle ready at ${HOST_DIR}"
    echo
    echo "Set for host runs:"
    echo "  export NOMOS_CIRCUITS=\"${HOST_DIR}\""
  fi

  cat <<'EOF'

Done.
- For Docker/compose: rebuild the image to bake the Linux bundle:
    scripts/build/build_test_image.sh
- For host runs (e.g., compose_runner): ensure NOMOS_CIRCUITS points to the host bundle above.
EOF
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  setup_circuits_stack::main "$@"
fi
