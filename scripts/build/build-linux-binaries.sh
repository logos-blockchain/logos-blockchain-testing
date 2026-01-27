#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../lib/common.sh"

build_linux_binaries::usage() {
  cat <<'EOF'
Usage: scripts/build/build-linux-binaries.sh [options]

Builds a Linux bundle via scripts/build/build-bundle.sh, then stages artifacts into:
  - testing-framework/assets/stack/bin

Options:
  --rev REV              logos-blockchain-node git revision to build (overrides LOGOS_BLOCKCHAIN_NODE_REV)
  --path DIR             use local logos-blockchain-node checkout (skip fetch/checkout)
  --features LIST        extra cargo features (comma-separated); base includes "testing"
  --docker-platform PLAT docker platform for the Linux build (e.g. linux/amd64, linux/arm64)
  --tar PATH             stage from an existing bundle tarball (skip build)
  --output PATH          where to write the bundle tarball when building (default: .tmp/nomos-binaries-linux-<version>.tar.gz)
  -h, --help             show help

Environment:
  VERSION                bundle version (default from versions.env)
  LOGOS_BLOCKCHAIN_CIRCUITS_VERSION legacy alias for VERSION (supported)
  LOGOS_BLOCKCHAIN_NODE_REV         default logos-blockchain-node revision (from versions.env)
EOF
}

build_linux_binaries::fail_with_usage() {
  echo "$1" >&2
  build_linux_binaries::usage
  exit 1
}

build_linux_binaries::load_env() {
  ROOT_DIR="$(common::repo_root)"
  export ROOT_DIR

  common::require_file "${ROOT_DIR}/versions.env"
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/versions.env"
  common::maybe_source "${ROOT_DIR}/paths.env"

  DEFAULT_VERSION="${VERSION:?Missing VERSION in versions.env}"
  VERSION="${VERSION:-${DEFAULT_VERSION}}"
  if [ -n "${LOGOS_BLOCKCHAIN_CIRCUITS_VERSION:-}" ]; then
    VERSION="${LOGOS_BLOCKCHAIN_CIRCUITS_VERSION}"
  fi
}

build_linux_binaries::parse_args() {
  REV_OVERRIDE=""
  PATH_OVERRIDE=""
  EXTRA_FEATURES=""
  DOCKER_PLATFORM=""
  OUTPUT_TAR=""
  INPUT_TAR=""

  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h|--help) build_linux_binaries::usage; exit 0 ;;
      --rev) REV_OVERRIDE="${2:-}"; shift 2 ;;
      --rev=*) REV_OVERRIDE="${1#*=}"; shift ;;
      --path) PATH_OVERRIDE="${2:-}"; shift 2 ;;
      --path=*) PATH_OVERRIDE="${1#*=}"; shift ;;
      --features) EXTRA_FEATURES="${2:-}"; shift 2 ;;
      --features=*) EXTRA_FEATURES="${1#*=}"; shift ;;
      --docker-platform) DOCKER_PLATFORM="${2:-}"; shift 2 ;;
      --docker-platform=*) DOCKER_PLATFORM="${1#*=}"; shift ;;
      --tar) INPUT_TAR="${2:-}"; shift 2 ;;
      --tar=*) INPUT_TAR="${1#*=}"; shift ;;
      --output|-o) OUTPUT_TAR="${2:-}"; shift 2 ;;
      --output=*|-o=*) OUTPUT_TAR="${1#*=}"; shift ;;
      *) build_linux_binaries::fail_with_usage "Unknown argument: $1" ;;
    esac
  done

  if [ -n "${REV_OVERRIDE}" ] && [ -n "${PATH_OVERRIDE}" ]; then
    build_linux_binaries::fail_with_usage "Use either --rev or --path, not both"
  fi
  if [ -n "${INPUT_TAR}" ] && [ ! -f "${INPUT_TAR}" ]; then
    build_linux_binaries::fail_with_usage "Bundle tarball not found: ${INPUT_TAR}"
  fi

  if [ -z "${OUTPUT_TAR}" ]; then
    OUTPUT_TAR="${ROOT_DIR}/.tmp/nomos-binaries-linux-${VERSION}.tar.gz"
  elif [[ "${OUTPUT_TAR}" != /* ]]; then
    OUTPUT_TAR="${ROOT_DIR}/${OUTPUT_TAR#./}"
  fi
}

build_linux_binaries::build_bundle_if_needed() {
  if [ -n "${INPUT_TAR}" ]; then
    BUNDLE_TAR="${INPUT_TAR}"
    return 0
  fi

  mkdir -p "$(dirname "${OUTPUT_TAR}")"
  BUILD_ARGS=(--platform linux --output "${OUTPUT_TAR}")
  if [ -n "${REV_OVERRIDE}" ]; then
    BUILD_ARGS+=(--rev "${REV_OVERRIDE}")
  elif [ -n "${PATH_OVERRIDE}" ]; then
    BUILD_ARGS+=(--path "${PATH_OVERRIDE}")
  fi
  if [ -n "${EXTRA_FEATURES}" ]; then
    BUILD_ARGS+=(--features "${EXTRA_FEATURES}")
  fi
  if [ -n "${DOCKER_PLATFORM}" ]; then
    BUILD_ARGS+=(--docker-platform "${DOCKER_PLATFORM}")
  fi

  echo "==> Building Linux bundle"
  VERSION="${VERSION}" "${ROOT_DIR}/scripts/build/build-bundle.sh" "${BUILD_ARGS[@]}"

  BUNDLE_TAR="${OUTPUT_TAR}"
}

build_linux_binaries::stage_from_bundle() {
  local tar_path="$1"
  local extract_dir
  extract_dir="$(common::tmpdir nomos-linux-bundle.XXXXXX)"
  cleanup() { rm -rf "${extract_dir}" 2>/dev/null || true; }
  trap cleanup EXIT

  echo "==> Extracting ${tar_path}"
  tar -xzf "${tar_path}" -C "${extract_dir}"

  local artifacts="${extract_dir}/artifacts"
  [ -f "${artifacts}/logos-blockchain-node" ] || common::die "Missing logos-blockchain-node in bundle: ${tar_path}"
  local bin_out="${ROOT_DIR}/testing-framework/assets/stack/bin"

  echo "==> Staging binaries to ${bin_out}"
  mkdir -p "${bin_out}"
  cp "${artifacts}/logos-blockchain-node" "${bin_out}/"
  # If the tarball was produced inside Docker, it might be root-owned on the host.
  chown -R "$(id -u)":"$(id -g)" "${bin_out}" 2>/dev/null || true
}

build_linux_binaries::main() {
  build_linux_binaries::load_env
  build_linux_binaries::parse_args "$@"
  build_linux_binaries::build_bundle_if_needed
  build_linux_binaries::stage_from_bundle "${BUNDLE_TAR}"

  echo
  echo "Binaries staged in ${ROOT_DIR}/testing-framework/assets/stack/bin"
  echo "Bundle tarball: ${BUNDLE_TAR}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  build_linux_binaries::main "$@"
fi
