#!/usr/bin/env bash
# Build Linux nomos-node/nomos-executor/nomos-cli binaries and stage them into
# testing-framework/assets/stack/bin along with the circuits bundle. This uses
# a Dockerized toolchain so it can be run from macOS as well.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -f "${ROOT_DIR}/paths.env" ]; then
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/paths.env"
fi
NOMOS_NODE_REV="${NOMOS_NODE_REV:-d2dd5a5084e1daef4032562c77d41de5e4d495f8}"
NOMOS_CIRCUITS_VERSION="${NOMOS_CIRCUITS_VERSION:-v0.3.1}"
NOMOS_BIN_PLATFORM="${NOMOS_BIN_PLATFORM:-linux/amd64}"

case "${NOMOS_BIN_PLATFORM}" in
  linux/amd64) CIRCUITS_PLATFORM="linux-x86_64" ;;
  linux/arm64) CIRCUITS_PLATFORM="linux-aarch64" ;;
  *) echo "Unsupported platform ${NOMOS_BIN_PLATFORM}. Use linux/amd64 or linux/arm64." >&2; exit 1 ;;
esac

echo "Workspace: ${ROOT_DIR}"
echo "Nomos node rev: ${NOMOS_NODE_REV}"
echo "Circuits version: ${NOMOS_CIRCUITS_VERSION}"

BIN_OUT="${ROOT_DIR}/testing-framework/assets/stack/bin"
KZG_DIR_REL="${NOMOS_KZG_DIR_REL:-testing-framework/assets/stack/kzgrs_test_params}"
CIRCUITS_OUT="${ROOT_DIR}/${KZG_DIR_REL}"
SRC_DIR="${ROOT_DIR}/.tmp/nomos-node-src"
CIRCUITS_DIR="${ROOT_DIR}/.tmp/nomos-circuits"

rm -rf "${CIRCUITS_OUT}"
mkdir -p "${BIN_OUT}" "${CIRCUITS_OUT}" "${SRC_DIR}" "${CIRCUITS_DIR}"

docker run --rm --platform "${NOMOS_BIN_PLATFORM}" \
  -v "${ROOT_DIR}:/workspace" \
  -w /workspace \
  -e NOMOS_NODE_REV="${NOMOS_NODE_REV}" \
  -e NOMOS_CIRCUITS_VERSION="${NOMOS_CIRCUITS_VERSION}" \
  -e NOMOS_CIRCUITS_PLATFORM="${CIRCUITS_PLATFORM}" \
  rust:1.91.0-slim-bookworm \
  bash -euo pipefail -c '
    apt-get update && apt-get install -y git clang llvm-dev libclang-dev pkg-config cmake libssl-dev rsync libgmp-dev libgomp1 nasm curl ca-certificates xz-utils
    cd /workspace
    RAPIDSNARK_BUILD_GMP=0 RAPIDSNARK_USE_ASM=OFF \
      ./scripts/setup-nomos-circuits.sh "${NOMOS_CIRCUITS_VERSION}" "/workspace/.tmp/nomos-circuits"

    if [ ! -d /workspace/.tmp/nomos-node-src/.git ]; then
      git clone https://github.com/logos-co/nomos-node.git /workspace/.tmp/nomos-node-src
    fi
    cd /workspace/.tmp/nomos-node-src
    git fetch --depth 1 origin "${NOMOS_NODE_REV}"
    git checkout "${NOMOS_NODE_REV}"
    git reset --hard
    git clean -fdx

    NOMOS_CIRCUITS=/workspace/.tmp/nomos-circuits \
      cargo build --features "testing" \
      -p nomos-node -p nomos-executor -p nomos-cli

    cp /workspace/.tmp/nomos-node-src/target/debug/nomos-node /workspace/testing-framework/assets/stack/bin/
    cp /workspace/.tmp/nomos-node-src/target/debug/nomos-executor /workspace/testing-framework/assets/stack/bin/
    cp /workspace/.tmp/nomos-node-src/target/debug/nomos-cli /workspace/testing-framework/assets/stack/bin/
    rsync -a /workspace/.tmp/nomos-circuits/ /workspace/testing-framework/assets/stack/kzgrs_test_params/
  '

# Ensure host ownership of staged artifacts.
chown -R "$(id -u)":"$(id -g)" "${BIN_OUT}" "${CIRCUITS_OUT}" "${SRC_DIR}" "${CIRCUITS_DIR}" 2>/dev/null || true

echo
echo "Binaries staged in ${BIN_OUT}:"
ls -l "${BIN_OUT}"
echo
echo "Circuits staged in ${CIRCUITS_OUT}"
