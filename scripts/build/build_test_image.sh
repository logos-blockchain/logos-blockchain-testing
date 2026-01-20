#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../lib/common.sh"

build_test_image::usage() {
  cat <<'USAGE'
Usage: scripts/build/build_test_image.sh [options]

Builds the compose/k8s test image (bakes in binaries + circuit assets).

Options:
  --tag TAG                 Docker image tag (default: logos-blockchain-testing:local; or env IMAGE_TAG)
  --version VERSION         Circuits release tag (default: versions.env VERSION)
  --dockerfile PATH         Dockerfile path (default: testing-framework/assets/stack/Dockerfile.runtime)
  --base-tag TAG            Base image tag (default: logos-blockchain-testing:base)
  --circuits-override PATH  Relative path (within repo) to circuits dir/file to bake (default: testing-framework/assets/stack/kzgrs_test_params)
  --circuits-platform NAME  Circuits platform identifier for downloads (default: auto; linux-x86_64 or linux-aarch64)
  --bundle-tar PATH         Bundle tar containing artifacts/{nomos-*,circuits} (default: .tmp/nomos-binaries-linux-<version>.tar.gz; or env NOMOS_BINARIES_TAR)
  --no-restore              Do not restore binaries/circuits from bundle tar (forces Dockerfile to build/download as needed)
  --print-config            Print resolved configuration and exit
  -h, --help                Show this help and exit

Env (legacy/compatible):
  IMAGE_TAG, VERSION, CIRCUITS_OVERRIDE, CIRCUITS_PLATFORM, COMPOSE_CIRCUITS_PLATFORM,
  NOMOS_BINARIES_TAR, NOMOS_KZG_DIR_REL
USAGE
}

build_test_image::fail() {
  common::die "$1"
}

build_test_image::load_env() {
  if [ -n "${ROOT_DIR:-}" ] && [ -f "${ROOT_DIR}/versions.env" ]; then
    : # Use provided ROOT_DIR.
  else
    ROOT_DIR="$(common::repo_root)"
  fi
  export ROOT_DIR

  common::require_file "${ROOT_DIR}/versions.env"
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/versions.env"
  common::maybe_source "${ROOT_DIR}/paths.env"

  DOCKERFILE_PATH_DEFAULT="${ROOT_DIR}/testing-framework/assets/stack/Dockerfile.runtime"
  BASE_DOCKERFILE_PATH_DEFAULT="${ROOT_DIR}/testing-framework/assets/stack/Dockerfile.base"
  IMAGE_TAG_DEFAULT="logos-blockchain-testing:local"
  BASE_IMAGE_TAG_DEFAULT="logos-blockchain-testing:base"

  VERSION_DEFAULT="${VERSION:?Missing VERSION in versions.env}"
  NOMOS_NODE_REV="${NOMOS_NODE_REV:?Missing NOMOS_NODE_REV in versions.env}"
}

build_test_image::detect_circuits_platform() {
  case "$(uname -m)" in
    x86_64) echo "linux-x86_64" ;;
    arm64|aarch64) echo "linux-aarch64" ;;
    *) echo "linux-x86_64" ;;
  esac
}

build_test_image::parse_args() {
  IMAGE_TAG="${IMAGE_TAG:-${IMAGE_TAG_DEFAULT}}"
  VERSION_OVERRIDE=""
  DOCKERFILE_PATH="${DOCKERFILE_PATH_DEFAULT}"
  BASE_DOCKERFILE_PATH="${BASE_DOCKERFILE_PATH_DEFAULT}"
  BASE_IMAGE_TAG="${BASE_IMAGE_TAG:-${BASE_IMAGE_TAG_DEFAULT}}"
  KZG_DIR_REL_DEFAULT="${NOMOS_KZG_DIR_REL:-testing-framework/assets/stack/kzgrs_test_params}"
  CIRCUITS_OVERRIDE="${CIRCUITS_OVERRIDE:-${KZG_DIR_REL_DEFAULT}}"
  CIRCUITS_PLATFORM="${CIRCUITS_PLATFORM:-${COMPOSE_CIRCUITS_PLATFORM:-}}"
  BUNDLE_TAR_PATH="${NOMOS_BINARIES_TAR:-}"
  NO_RESTORE=0
  PRINT_CONFIG=0

  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h|--help) build_test_image::usage; exit 0 ;;
      --tag=*) IMAGE_TAG="${1#*=}"; shift ;;
      --tag) IMAGE_TAG="${2:-}"; shift 2 ;;
      --version=*) VERSION_OVERRIDE="${1#*=}"; shift ;;
      --version) VERSION_OVERRIDE="${2:-}"; shift 2 ;;
      --dockerfile=*) DOCKERFILE_PATH="${1#*=}"; shift ;;
      --dockerfile) DOCKERFILE_PATH="${2:-}"; shift 2 ;;
      --base-tag=*) BASE_IMAGE_TAG="${1#*=}"; shift ;;
      --base-tag) BASE_IMAGE_TAG="${2:-}"; shift 2 ;;
      --circuits-override=*) CIRCUITS_OVERRIDE="${1#*=}"; shift ;;
      --circuits-override) CIRCUITS_OVERRIDE="${2:-}"; shift 2 ;;
      --circuits-platform=*) CIRCUITS_PLATFORM="${1#*=}"; shift ;;
      --circuits-platform) CIRCUITS_PLATFORM="${2:-}"; shift 2 ;;
      --bundle-tar=*) BUNDLE_TAR_PATH="${1#*=}"; shift ;;
      --bundle-tar) BUNDLE_TAR_PATH="${2:-}"; shift 2 ;;
      --no-restore) NO_RESTORE=1; shift ;;
      --print-config) PRINT_CONFIG=1; shift ;;
      *) build_test_image::fail "Unknown argument: $1" ;;
    esac
  done

  if [ -n "${VERSION_OVERRIDE}" ]; then
    VERSION="${VERSION_OVERRIDE}"
  else
    VERSION="${VERSION_DEFAULT}"
  fi

  if [ -z "${CIRCUITS_PLATFORM}" ]; then
    CIRCUITS_PLATFORM="$(build_test_image::detect_circuits_platform)"
  fi

  BIN_DST="${ROOT_DIR}/testing-framework/assets/stack/bin"
  KZG_DIR_REL="${KZG_DIR_REL_DEFAULT}"
  CIRCUITS_DIR_HOST="${ROOT_DIR}/${KZG_DIR_REL}"

  DEFAULT_LINUX_TAR="${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"
  TAR_PATH="${BUNDLE_TAR_PATH:-${DEFAULT_LINUX_TAR}}"
}

build_test_image::print_config() {
  echo "Workspace root: ${ROOT_DIR}"
  echo "Image tag: ${IMAGE_TAG}"
  echo "Dockerfile: ${DOCKERFILE_PATH}"
  echo "Base image tag: ${BASE_IMAGE_TAG}"
  echo "Base Dockerfile: ${BASE_DOCKERFILE_PATH}"
  echo "Logos node rev: ${NOMOS_NODE_REV}"
  echo "Circuits override: ${CIRCUITS_OVERRIDE:-<none>}"
  echo "Circuits version (download fallback): ${VERSION}"
  echo "Circuits platform: ${CIRCUITS_PLATFORM}"
  echo "Host circuits dir: ${CIRCUITS_DIR_HOST}"
  echo "Binaries dir: ${BIN_DST}"
  echo "Bundle tar (if used): ${TAR_PATH}"
  echo "Restore from tar: $([ "${NO_RESTORE}" -eq 1 ] && echo "disabled" || echo "enabled")"
}

build_test_image::have_host_binaries() {
  # Preserve existing behavior: only require logos-blockchain-node on the host.
  # If logos-blockchain-cli is missing, the Dockerfile can still build it from source.
  [ -x "${BIN_DST}/logos-blockchain-node" ]
}

build_test_image::restore_from_bundle() {
  [ -f "${TAR_PATH}" ] || build_test_image::fail "Prebuilt binaries missing and bundle tar not found at ${TAR_PATH}"

  echo "==> Restoring binaries/circuits from ${TAR_PATH}"
  local tmp_extract
  tmp_extract="$(common::tmpdir nomos-bundle-extract.XXXXXX)"
  trap "rm -rf -- '${tmp_extract}'" RETURN

  tar -xzf "${TAR_PATH}" -C "${tmp_extract}"
  local artifacts="${tmp_extract}/artifacts"

  for bin in logos-blockchain-node logos-blockchain-cli; do
    [ -f "${artifacts}/${bin}" ] || build_test_image::fail "Bundle ${TAR_PATH} missing artifacts/${bin}"
  done

  mkdir -p "${BIN_DST}"
  cp "${artifacts}/logos-blockchain-node" "${artifacts}/logos-blockchain-cli" "${BIN_DST}/"
  chmod +x "${BIN_DST}/logos-blockchain-node" "${BIN_DST}/logos-blockchain-cli" || true

  if [ -d "${artifacts}/circuits" ]; then
    mkdir -p "${CIRCUITS_DIR_HOST}"
    if command -v rsync >/dev/null 2>&1; then
      rsync -a --delete "${artifacts}/circuits/" "${CIRCUITS_DIR_HOST}/"
    else
      cp -a "${artifacts}/circuits/." "${CIRCUITS_DIR_HOST}/"
    fi
  fi
}

build_test_image::maybe_restore_assets() {
  if [ "${NO_RESTORE}" -eq 1 ]; then
    return 0
  fi
  if build_test_image::have_host_binaries; then
    return 0
  fi
  build_test_image::restore_from_bundle
}

build_test_image::docker_build() {
  command -v docker >/dev/null 2>&1 || build_test_image::fail "docker not found in PATH"
  [ -f "${DOCKERFILE_PATH}" ] || build_test_image::fail "Dockerfile not found: ${DOCKERFILE_PATH}"

  [ -f "${BASE_DOCKERFILE_PATH}" ] || build_test_image::fail "Base Dockerfile not found: ${BASE_DOCKERFILE_PATH}"

  local host_platform=""
  local target_platform=""
  case "$(uname -m)" in
    x86_64) host_platform="linux/amd64" ;;
    arm64|aarch64) host_platform="linux/arm64" ;;
  esac
  case "${CIRCUITS_PLATFORM}" in
    linux-x86_64) target_platform="linux/amd64" ;;
    linux-aarch64) target_platform="linux/arm64" ;;
  esac

  local -a base_build_args=(
    -f "${BASE_DOCKERFILE_PATH}"
    -t "${BASE_IMAGE_TAG}"
    --build-arg "NOMOS_NODE_REV=${NOMOS_NODE_REV}"
    --build-arg "CIRCUITS_PLATFORM=${CIRCUITS_PLATFORM}"
    --build-arg "VERSION=${VERSION}"
    "${ROOT_DIR}"
  )

  if [ -n "${CIRCUITS_OVERRIDE}" ]; then
    base_build_args+=(--build-arg "CIRCUITS_OVERRIDE=${CIRCUITS_OVERRIDE}")
  fi
  if [ -n "${host_platform}" ] && [ -n "${target_platform}" ] && [ "${host_platform}" != "${target_platform}" ]; then
    base_build_args+=(--platform "${target_platform}")
    base_build_args+=(--build-arg "RAPIDSNARK_FORCE_REBUILD=1")
  fi

  printf "Running:"
  printf " %q" docker build "${base_build_args[@]}"
  echo
  docker build "${base_build_args[@]}"

  local -a final_build_args=(
    -f "${DOCKERFILE_PATH}"
    -t "${IMAGE_TAG}"
    --build-arg "BASE_IMAGE=${BASE_IMAGE_TAG}"
    "${ROOT_DIR}"
  )
  if [ -n "${host_platform}" ] && [ -n "${target_platform}" ] && [ "${host_platform}" != "${target_platform}" ]; then
    final_build_args+=(--platform "${target_platform}")
  fi

  printf "Running:"
  printf " %q" docker build "${final_build_args[@]}"
  echo
  docker build "${final_build_args[@]}"
}

build_test_image::main() {
  build_test_image::load_env
  build_test_image::parse_args "$@"

  if [ "${PRINT_CONFIG}" -eq 1 ]; then
    build_test_image::print_config
    exit 0
  fi

  build_test_image::print_config
  build_test_image::maybe_restore_assets
  build_test_image::docker_build

  cat <<EOF

Build complete.
- Use this image in k8s/compose by exporting NOMOS_TESTNET_IMAGE=${IMAGE_TAG}
- Circuits source: ${CIRCUITS_OVERRIDE:-download ${VERSION}}
EOF
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  build_test_image::main "$@"
fi
