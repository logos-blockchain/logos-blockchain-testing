#!/usr/bin/env bash
set -euo pipefail

# Builds the testnet image with circuits. Prefers a local circuits bundle
# (tests/kzgrs/kzgrs_test_params) or a custom override; otherwise downloads
# from logos-co/nomos-circuits.

# Always run under bash; bail out if someone invokes via sh.
if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
# shellcheck disable=SC1091
. "${ROOT_DIR}/scripts/lib/common.sh"

common::require_file "${ROOT_DIR}/versions.env"
# shellcheck disable=SC1091
. "${ROOT_DIR}/versions.env"
common::maybe_source "${ROOT_DIR}/paths.env"

DOCKERFILE_PATH="${ROOT_DIR}/testing-framework/assets/stack/Dockerfile"
IMAGE_TAG="${IMAGE_TAG:-logos-blockchain-testing:local}"
DEFAULT_VERSION="${VERSION:-v0.3.1}"
VERSION="${VERSION:-${DEFAULT_VERSION}}"
KZG_DIR_REL="${NOMOS_KZG_DIR_REL:-testing-framework/assets/stack/kzgrs_test_params}"
CIRCUITS_DIR_HOST="${ROOT_DIR}/${KZG_DIR_REL}"
CIRCUITS_OVERRIDE="${CIRCUITS_OVERRIDE:-${KZG_DIR_REL}}"
CIRCUITS_PLATFORM="${CIRCUITS_PLATFORM:-${COMPOSE_CIRCUITS_PLATFORM:-}}"
if [ -z "${CIRCUITS_PLATFORM}" ]; then
  case "$(uname -m)" in
    x86_64) CIRCUITS_PLATFORM="linux-x86_64" ;;
    arm64|aarch64) CIRCUITS_PLATFORM="linux-aarch64" ;;
    *) CIRCUITS_PLATFORM="linux-x86_64" ;;
  esac
fi
NOMOS_NODE_REV="${NOMOS_NODE_REV:?Missing NOMOS_NODE_REV in versions.env or env}"

echo "Workspace root: ${ROOT_DIR}"
echo "Image tag: ${IMAGE_TAG}"
echo "Circuits override: ${CIRCUITS_OVERRIDE:-<none>}"
echo "Circuits version (fallback download): ${VERSION}"
echo "Circuits platform: ${CIRCUITS_PLATFORM}"
echo "Bundle tar (if used): ${NOMOS_BINARIES_TAR:-<default> ${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz}"

# If prebuilt binaries are missing, restore them from a bundle tarball instead of
# rebuilding nomos inside the image.
BIN_DST="${ROOT_DIR}/testing-framework/assets/stack/bin"
DEFAULT_LINUX_TAR="${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"
TAR_PATH="${NOMOS_BINARIES_TAR:-${DEFAULT_LINUX_TAR}}"

if [ ! -x "${BIN_DST}/nomos-node" ] || [ ! -x "${BIN_DST}/nomos-executor" ]; then
  if [ -f "${TAR_PATH}" ]; then
    echo "Restoring binaries/circuits from ${TAR_PATH}"
    tmp_extract="$(common::tmpdir nomos-bundle-extract.XXXXXX)"
    tar -xzf "${TAR_PATH}" -C "${tmp_extract}"
    if [ -f "${tmp_extract}/artifacts/nomos-node" ] && [ -f "${tmp_extract}/artifacts/nomos-executor" ]; then
      mkdir -p "${BIN_DST}"
      cp "${tmp_extract}/artifacts/nomos-node" "${tmp_extract}/artifacts/nomos-executor" "${tmp_extract}/artifacts/nomos-cli" "${BIN_DST}/"
    else
      common::die "Bundle ${TAR_PATH} missing binaries under artifacts/"
    fi
    if [ -d "${tmp_extract}/artifacts/circuits" ]; then
      mkdir -p "${CIRCUITS_DIR_HOST}"
      if command -v rsync >/dev/null 2>&1; then
        rsync -a --delete "${tmp_extract}/artifacts/circuits/" "${CIRCUITS_DIR_HOST}/"
      else
        cp -a "${tmp_extract}/artifacts/circuits/." "${CIRCUITS_DIR_HOST}/"
      fi
    fi
    rm -rf "${tmp_extract}"
  else
    common::die "Prebuilt binaries missing and bundle tar not found at ${TAR_PATH}"
  fi
fi

build_args=(
  -f "${DOCKERFILE_PATH}"
  -t "${IMAGE_TAG}"
  --build-arg "NOMOS_NODE_REV=${NOMOS_NODE_REV}"
  --build-arg "CIRCUITS_PLATFORM=${CIRCUITS_PLATFORM}"
  "${ROOT_DIR}"
)

# Pass override/version args to the Docker build.
if [ -n "${CIRCUITS_OVERRIDE}" ]; then
  build_args+=(--build-arg "CIRCUITS_OVERRIDE=${CIRCUITS_OVERRIDE}")
fi
build_args+=(--build-arg "VERSION=${VERSION}")

echo "Running: docker build ${build_args[*]}"
docker build "${build_args[@]}"

cat <<EOF

Build complete.
- Use this image in k8s/compose by exporting NOMOS_TESTNET_IMAGE=${IMAGE_TAG}
- Circuits source: ${CIRCUITS_OVERRIDE:-download ${VERSION}}
EOF
