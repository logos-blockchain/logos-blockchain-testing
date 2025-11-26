#!/bin/bash
set -euo pipefail

# Builds the testnet image with circuits. Prefers a local circuits bundle
# (tests/kzgrs/kzgrs_test_params) or a custom override; otherwise downloads
# from logos-co/nomos-circuits.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE_TAG="${IMAGE_TAG:-nomos-testnet:local}"
VERSION="${VERSION:-v0.3.1}"
CIRCUITS_OVERRIDE="${CIRCUITS_OVERRIDE:-tests/kzgrs/kzgrs_test_params}"

echo "Workspace root: ${ROOT_DIR}"
echo "Image tag: ${IMAGE_TAG}"
echo "Circuits override: ${CIRCUITS_OVERRIDE:-<none>}"
echo "Circuits version (fallback download): ${VERSION}"

build_args=(
  -f "${ROOT_DIR}/testnet/Dockerfile"
  -t "${IMAGE_TAG}"
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
