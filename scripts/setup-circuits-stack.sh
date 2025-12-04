#!/usr/bin/env bash
set -euo pipefail

# One-stop helper to prepare circuits for both the Docker image (Linux/x86_64)
# and the host (for witness generators). It populates
# testing-framework/assets/stack/kzgrs_test_params with a Linux bundle for the
# image, and if the host is not Linux/x86_64, it also fetches a host-native
# bundle and tells you where to point NOMOS_CIRCUITS.
#
# Usage: scripts/setup-circuits-stack.sh [VERSION]
#   VERSION defaults to v0.3.1
#
# Env overrides:
#   STACK_DIR   - where to place the Linux bundle (default: testing-framework/assets/stack/kzgrs_test_params)
#   HOST_DIR    - where to place the host bundle (default: .tmp/nomos-circuits-host)
#   NOMOS_CIRCUITS_PLATFORM - force host platform (e.g., macos-aarch64)
#   NOMOS_CIRCUITS_REBUILD_RAPIDSNARK - set to 1 to force rebuild (not needed for mac arm/x86 bundles)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-v0.3.1}"
STACK_DIR="${STACK_DIR:-${ROOT_DIR}/testing-framework/assets/stack/kzgrs_test_params}"
HOST_DIR="${HOST_DIR:-${ROOT_DIR}/.tmp/nomos-circuits-host}"
NOMOS_NODE_REV="${NOMOS_NODE_REV:-2f60a0372c228968c3526c341ebc7e58bbd178dd}"

detect_platform() {
  local os arch
  case "$(uname -s)" in
    Linux*) os="linux" ;;
    Darwin*) os="macos" ;;
    MINGW*|MSYS*|CYGWIN*) os="windows" ;;
    *) echo "Unsupported OS: $(uname -s)" >&2; exit 1 ;;
  esac

  case "$(uname -m)" in
    x86_64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) echo "Unsupported arch: $(uname -m)" >&2; exit 1 ;;
  esac

  echo "${os}-${arch}"
}

fetch_bundle() {
  local platform="$1"
  local dest="$2"
  local rebuild="${3:-0}"

  rm -rf "$dest"
  mkdir -p "$dest"

  NOMOS_CIRCUITS_PLATFORM="$platform" \
  NOMOS_CIRCUITS_REBUILD_RAPIDSNARK="$rebuild" \
    "${ROOT_DIR}/scripts/setup-nomos-circuits.sh" "$VERSION" "$dest"
}

fetch_kzg_params() {
  local dest_dir="$1"
  local dest_file="${dest_dir}/kzgrs_test_params"
  local url="https://raw.githubusercontent.com/logos-co/nomos-node/${NOMOS_NODE_REV}/tests/kzgrs/kzgrs_test_params"

  echo "Fetching KZG parameters from ${url}"
  curl -fsSL "$url" -o "$dest_file"
}

echo "Preparing circuits (version ${VERSION})"
echo "Workspace: ${ROOT_DIR}"

LINUX_PLATFORM="linux-x86_64"

echo "Installing Linux bundle for Docker image into ${STACK_DIR}"
tmp_linux="$(mktemp -d)"
fetch_bundle "$LINUX_PLATFORM" "$tmp_linux" 0
rm -rf "$STACK_DIR"
mkdir -p "$STACK_DIR"
cp -R "${tmp_linux}/." "$STACK_DIR/"
rm -rf "$tmp_linux"
fetch_kzg_params "$STACK_DIR"
echo "Linux bundle ready at ${STACK_DIR}"

host_platform="${NOMOS_CIRCUITS_PLATFORM:-$(detect_platform)}"
if [[ "$host_platform" == "$LINUX_PLATFORM" ]]; then
  echo "Host platform ${host_platform} matches Linux bundle; host can reuse ${STACK_DIR}"
  echo "Export if you want to be explicit:"
  echo "  export NOMOS_CIRCUITS=\"${STACK_DIR}\""
else
  echo "Host platform detected: ${host_platform}; installing host-native bundle into ${HOST_DIR}"
  fetch_bundle "$host_platform" "$HOST_DIR" "${NOMOS_CIRCUITS_REBUILD_RAPIDSNARK:-0}"
  fetch_kzg_params "$HOST_DIR"
  echo "Host bundle ready at ${HOST_DIR}"
  echo
  echo "Set for host runs:"
  echo "  export NOMOS_CIRCUITS=\"${HOST_DIR}\""
fi

cat <<'EOF'

Done.
- For Docker/compose: rebuild the image to bake the Linux bundle:
    testing-framework/assets/stack/scripts/build_test_image.sh
- For host runs (e.g., compose_runner): ensure NOMOS_CIRCUITS points to the host bundle above.
EOF
