#!/bin/bash
set -euo pipefail

# Builds the testnet image with circuits. Prefers a local circuits bundle
# (tests/kzgrs/kzgrs_test_params) or a custom override; otherwise downloads
# from logos-co/nomos-circuits.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
if [ -f "${ROOT_DIR}/versions.env" ]; then
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/versions.env"
fi
if [ -f "${ROOT_DIR}/paths.env" ]; then
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/paths.env"
fi
DOCKERFILE_PATH="${ROOT_DIR}/testing-framework/assets/stack/Dockerfile"
IMAGE_TAG="${IMAGE_TAG:-nomos-testnet:local}"
VERSION="${VERSION:-v0.3.1}"
KZG_DIR_REL="${NOMOS_KZG_DIR_REL:-testing-framework/assets/stack/kzgrs_test_params}"
CIRCUITS_OVERRIDE="${CIRCUITS_OVERRIDE:-${KZG_DIR_REL}}"
NOMOS_NODE_REV="${NOMOS_NODE_REV:-d2dd5a5084e1daef4032562c77d41de5e4d495f8}"

echo "Workspace root: ${ROOT_DIR}"
echo "Image tag: ${IMAGE_TAG}"
echo "Circuits override: ${CIRCUITS_OVERRIDE:-<none>}"
echo "Circuits version (fallback download): ${VERSION}"

build_args=(
  -f "${DOCKERFILE_PATH}"
  -t "${IMAGE_TAG}"
  --build-arg "NOMOS_NODE_REV=${NOMOS_NODE_REV}"
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
