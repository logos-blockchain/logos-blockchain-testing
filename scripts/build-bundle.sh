#!/usr/bin/env bash
set -euo pipefail

# Build a nomos-binaries.tar.gz for the specified platform.
#
# Usage: scripts/build-bundle.sh [--platform host|linux] [--output PATH]
#   --platform   Target platform for binaries (default: host)
#   --output     Output path for the tarball (default: .tmp/nomos-binaries-<platform>-<version>.tar.gz)

usage() {
  cat <<'EOF'
Usage: scripts/build-bundle.sh [--platform host|linux] [--output PATH]

Options:
  --platform   Target platform for binaries (default: host)
  --output     Output path for the tarball (default: .tmp/nomos-binaries-<platform>-<version>.tar.gz)

Notes:
  - For compose/k8s, use platform=linux. If running on macOS, this script will
    run inside a Linux Docker container to produce Linux binaries.
  - VERSION and NOMOS_NODE_REV env vars are honored (defaults align with run-examples.sh).
EOF
}

fail() { echo "$1" >&2; exit 1; }

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage; exit 0
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEFAULT_VERSION="v0.3.1"
DEFAULT_NODE_REV="d2dd5a5084e1daef4032562c77d41de5e4d495f8"
PLATFORM="host"
OUTPUT=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --platform|-p)
      PLATFORM="${2:-}"; shift 2 ;;
    --output|-o)
      OUTPUT="${2:-}"; shift 2 ;;
    *) fail "Unknown argument: $1" ;;
  esac
done

case "$PLATFORM" in
  host|linux) ;;
  *) fail "--platform must be host or linux" ;;
esac

VERSION="${VERSION:-${DEFAULT_VERSION}}"
NOMOS_NODE_REV="${NOMOS_NODE_REV:-${DEFAULT_NODE_REV}}"
if [ -z "${OUTPUT}" ]; then
  OUTPUT="${ROOT_DIR}/.tmp/nomos-binaries-${PLATFORM}-${VERSION}.tar.gz"
fi

if [ "$PLATFORM" = "linux" ] && [ "$(uname -s)" != "Linux" ] && [ -z "${BUNDLE_IN_CONTAINER:-}" ]; then
  # Re-run inside a Linux container to produce Linux binaries.
  if ! command -v docker >/dev/null 2>&1; then
    fail "Docker is required to build a Linux bundle from non-Linux host"
  fi
  echo "==> Building Linux bundle inside Docker"
  mkdir -p "${ROOT_DIR}/.tmp/cargo-linux" "${ROOT_DIR}/.tmp/nomos-node-linux-target"
  docker run --rm \
    -e VERSION="$VERSION" \
    -e NOMOS_NODE_REV="$NOMOS_NODE_REV" \
    -e NOMOS_CIRCUITS="/workspace/.tmp/nomos-circuits-linux" \
    -e STACK_DIR="/workspace/.tmp/nomos-circuits-linux" \
    -e HOST_DIR="/workspace/.tmp/nomos-circuits-linux" \
    -e BUNDLE_IN_CONTAINER=1 \
    -e CARGO_HOME=/workspace/.tmp/cargo-linux \
    -e CARGO_TARGET_DIR=/workspace/.tmp/nomos-node-linux-target \
    -v "${ROOT_DIR}/.tmp/cargo-linux":/workspace/.tmp/cargo-linux \
    -v "${ROOT_DIR}/.tmp/nomos-node-linux-target":/workspace/.tmp/nomos-node-linux-target \
    -v "$ROOT_DIR":/workspace \
    -w /workspace \
    rust:1.80-bullseye \
    bash -c "apt-get update && apt-get install -y clang llvm-dev libclang-dev pkg-config cmake libssl-dev rsync libgmp10 libgmp-dev libgomp1 nasm && ./scripts/build-bundle.sh --platform linux --output /workspace/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"
  exit 0
fi

echo "==> Preparing circuits (version ${VERSION})"
if [ "$PLATFORM" = "host" ]; then
  CIRCUITS_DIR="${ROOT_DIR}/.tmp/nomos-circuits-host"
  NODE_SRC="${ROOT_DIR}/.tmp/nomos-node-host-src"
  NODE_TARGET="${ROOT_DIR}/.tmp/nomos-node-host-target"
else
  CIRCUITS_DIR="${ROOT_DIR}/.tmp/nomos-circuits-linux"
  NODE_SRC="${ROOT_DIR}/.tmp/nomos-node-linux-src"
  NODE_TARGET="${ROOT_DIR}/.tmp/nomos-node-linux-target"
fi
export NOMOS_CIRCUITS="${CIRCUITS_DIR}"
mkdir -p "${ROOT_DIR}/.tmp" "${CIRCUITS_DIR}"
STACK_DIR="${CIRCUITS_DIR}" HOST_DIR="${CIRCUITS_DIR}" \
  "${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}" </dev/null

NODE_BIN="${NODE_TARGET}/debug/nomos-node"
EXEC_BIN="${NODE_TARGET}/debug/nomos-executor"
CLI_BIN="${NODE_TARGET}/debug/nomos-cli"

echo "==> Building host binaries (platform=${PLATFORM})"
mkdir -p "${NODE_SRC}"
if [ ! -d "${NODE_SRC}/.git" ]; then
  git clone https://github.com/logos-co/nomos-node.git "${NODE_SRC}"
fi
(
  cd "${NODE_SRC}"
  git fetch --depth 1 origin "${NOMOS_NODE_REV}"
  git checkout "${NOMOS_NODE_REV}"
  git reset --hard
  git clean -fdx
  RUSTFLAGS='--cfg feature="pol-dev-mode"' NOMOS_CIRCUITS="${CIRCUITS_DIR}" \
    cargo build --features testing \
    -p nomos-node -p nomos-executor -p nomos-cli \
    --target-dir "${NODE_TARGET}"
)

echo "==> Packaging bundle"
bundle_dir="${ROOT_DIR}/.tmp/nomos-bundle"
rm -rf "${bundle_dir}"
mkdir -p "${bundle_dir}/artifacts/circuits"
cp -a "${CIRCUITS_DIR}/." "${bundle_dir}/artifacts/circuits/"
mkdir -p "${bundle_dir}/artifacts"
cp "${NODE_BIN}" "${bundle_dir}/artifacts/"
cp "${EXEC_BIN}" "${bundle_dir}/artifacts/"
cp "${CLI_BIN}" "${bundle_dir}/artifacts/"

mkdir -p "$(dirname "${OUTPUT}")"
if tar --help 2>/dev/null | grep -q -- '--no-mac-metadata'; then
  tar --no-mac-metadata --no-xattrs -czf "${OUTPUT}" -C "${bundle_dir}" artifacts
elif tar --help 2>/dev/null | grep -q -- '--no-xattrs'; then
  tar --no-xattrs -czf "${OUTPUT}" -C "${bundle_dir}" artifacts
else
  tar -czf "${OUTPUT}" -C "${bundle_dir}" artifacts
fi
echo "Bundle created at ${OUTPUT}"
