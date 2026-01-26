#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# shellcheck disable=SC1091
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../lib/common.sh"

readonly DOCKER_RUST_IMAGE="rust:1.80-bullseye"
declare -ar DOCKER_APT_PACKAGES=(
  clang
  llvm-dev
  libclang-dev
  pkg-config
  cmake
  libssl-dev
  rsync
  libgmp10
  libgmp-dev
  libgomp1
  nasm
)

build_bundle::usage() {
  cat <<'USAGE'
Usage: scripts/build/build-bundle.sh [--platform host|linux] [--output PATH]

Options:
  --platform        Target platform for binaries (default: host)
  --output          Output path for the tarball (default: .tmp/nomos-binaries-<platform>-<version>.tar.gz)
  --rev             logos-blockchain-node git revision to build (overrides NOMOS_NODE_REV)
  --path            Use local logos-blockchain-node checkout at DIR (skip fetch/checkout)
  --features        Extra cargo features to enable (comma-separated); base always includes "testing"
  --docker-platform Docker platform for Linux bundle when running on non-Linux host (default: auto; linux/arm64 on Apple silicon Docker Desktop, else linux/amd64)

Notes:
  - For compose/k8s, use platform=linux. If running on macOS, this script will
    run inside a Linux Docker container to produce Linux binaries.
  - On Apple silicon, Docker defaults to linux/arm64; for compose/k8s you likely
    want linux/amd64 (the default here). Override with --docker-platform.
  - VERSION, NOMOS_NODE_REV, and optional NOMOS_NODE_PATH env vars are honored (defaults align with run-examples.sh).
USAGE
}

build_bundle::fail() {
  echo "$1" >&2
  exit 1
}

build_bundle::apply_nomos_node_patches() {
  local node_src="$1"

  local apply="${NOMOS_NODE_APPLY_PATCHES:-1}"
  if [ "${apply}" = "0" ]; then
    return 0
  fi

  local patch_dir="${NOMOS_NODE_PATCH_DIR:-${ROOT_DIR}/patches/logos-blockchain-node}"
  if [ ! -d "${patch_dir}" ]; then
    return 0
  fi

  local level="${NOMOS_NODE_PATCH_LEVEL:-}"
  if [ -z "${level}" ]; then
    level="all"
  fi

  shopt -s nullglob
  local -a patches=("${patch_dir}"/*.patch)
  shopt -u nullglob
  if [ "${#patches[@]}" -eq 0 ]; then
    return 0
  fi

  echo "==> Applying logos-blockchain-node patches from ${patch_dir} (level=${level})"
  local patch base phase
  for patch in "${patches[@]}"; do
    base="$(basename "${patch}")"
    phase=""
    if [[ "${base}" =~ phase([0-9]+) ]]; then
      phase="${BASH_REMATCH[1]}"
    fi
    if [ "${level}" != "all" ] && [ "${level}" != "ALL" ]; then
      if ! [[ "${level}" =~ ^[0-9]+$ ]]; then
        build_bundle::fail "Invalid NOMOS_NODE_PATCH_LEVEL: ${level} (expected integer or 'all')"
      fi
      if [ -n "${phase}" ] && [ "${phase}" -gt "${level}" ]; then
        continue
      fi
    fi

    git -C "${node_src}" apply --whitespace=nowarn "${patch}"
  done
}

build_bundle::load_env() {
  ROOT_DIR="$(common::repo_root)"
  export ROOT_DIR

  common::require_file "${ROOT_DIR}/versions.env"
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/versions.env"

  DEFAULT_VERSION="${VERSION:?Missing VERSION in versions.env}"
  DEFAULT_NODE_REV="${NOMOS_NODE_REV:-}"
  DEFAULT_NODE_PATH="${NOMOS_NODE_PATH:-}"

  NOMOS_EXTRA_FEATURES="${NOMOS_EXTRA_FEATURES:-}"
  DOCKER_PLATFORM="${NOMOS_BUNDLE_DOCKER_PLATFORM:-${NOMOS_BIN_PLATFORM:-}}"
  BUNDLE_RUSTUP_TOOLCHAIN="${BUNDLE_RUSTUP_TOOLCHAIN:-}"

  if [ -z "${BUNDLE_RUSTUP_TOOLCHAIN}" ] && command -v rustup >/dev/null 2>&1 && [ -f "${ROOT_DIR}/rust-toolchain.toml" ]; then
    BUNDLE_RUSTUP_TOOLCHAIN="$(awk -F '\"' '/^[[:space:]]*channel[[:space:]]*=/{print $2; exit}' "${ROOT_DIR}/rust-toolchain.toml")"
  fi
}

build_bundle::default_docker_platform() {
  if [ -n "${DOCKER_PLATFORM}" ]; then
    return 0
  fi
  if ! command -v docker >/dev/null 2>&1; then
    return 0
  fi
  local docker_arch
  docker_arch="$(docker version --format '{{.Server.Arch}}' 2>/dev/null || true)"
  case "${docker_arch}" in
    arm64|aarch64) DOCKER_PLATFORM="linux/arm64" ;;
    amd64|x86_64) DOCKER_PLATFORM="linux/amd64" ;;
    *) DOCKER_PLATFORM="linux/amd64" ;;
  esac
}

build_bundle::parse_args() {
  PLATFORM="host"
  OUTPUT=""
  REV_OVERRIDE=""
  PATH_OVERRIDE=""

  if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
    build_bundle::usage
    exit 0
  fi

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --platform=*|-p=*) PLATFORM="${1#*=}"; shift ;;
      --platform|-p) PLATFORM="${2:-}"; shift 2 ;;
      --output=*|-o=*) OUTPUT="${1#*=}"; shift ;;
      --output|-o) OUTPUT="${2:-}"; shift 2 ;;
      --rev=*) REV_OVERRIDE="${1#*=}"; shift ;;
      --rev) REV_OVERRIDE="${2:-}"; shift 2 ;;
      --path=*) PATH_OVERRIDE="${1#*=}"; shift ;;
      --path) PATH_OVERRIDE="${2:-}"; shift 2 ;;
      --features=*) NOMOS_EXTRA_FEATURES="${1#*=}"; shift ;;
      --features) NOMOS_EXTRA_FEATURES="${2:-}"; shift 2 ;;
      --docker-platform=*) DOCKER_PLATFORM="${1#*=}"; shift ;;
      --docker-platform) DOCKER_PLATFORM="${2:-}"; shift 2 ;;
      *) build_bundle::fail "Unknown argument: $1" ;;
    esac
  done
}

build_bundle::validate_and_finalize() {
  case "${PLATFORM}" in
    host|linux) ;;
    *) build_bundle::fail "--platform must be host or linux" ;;
  esac

  VERSION="${DEFAULT_VERSION}"

  if [ -n "${REV_OVERRIDE}" ] && [ -n "${PATH_OVERRIDE}" ]; then
    build_bundle::fail "Use either --rev or --path, not both"
  fi
  if [ -z "${REV_OVERRIDE}" ] && [ -z "${PATH_OVERRIDE}" ] && [ -z "${DEFAULT_NODE_REV}" ] && [ -z "${DEFAULT_NODE_PATH}" ]; then
    build_bundle::fail "Provide --rev, --path, or set NOMOS_NODE_REV/NOMOS_NODE_PATH in versions.env"
  fi
  NOMOS_NODE_REV="${REV_OVERRIDE:-${DEFAULT_NODE_REV}}"
  NOMOS_NODE_PATH="${PATH_OVERRIDE:-${DEFAULT_NODE_PATH}}"
  export NOMOS_NODE_REV NOMOS_NODE_PATH

  build_bundle::default_docker_platform
  DOCKER_PLATFORM="${DOCKER_PLATFORM:-linux/amd64}"

  # Normalize OUTPUT to an absolute path under the workspace.
  if [ -z "${OUTPUT}" ]; then
    OUTPUT="${ROOT_DIR}/.tmp/nomos-binaries-${PLATFORM}-${VERSION}.tar.gz"
  elif [[ "${OUTPUT}" != /* ]]; then
    OUTPUT="${ROOT_DIR}/${OUTPUT#./}"
  fi
  echo "Bundle output: ${OUTPUT}"
}

build_bundle::clean_cargo_linux_cache() {
  rm -rf "${ROOT_DIR}/.tmp/cargo-linux/registry" "${ROOT_DIR}/.tmp/cargo-linux/git"
}

build_bundle::docker_platform_suffix() {
  # Map a docker platform string (e.g. linux/amd64) to a filesystem-safe suffix
  # used for arch-specific target dirs, to avoid mixing build artifacts between
  # different container architectures.
  local platform="${1:-}"
  if [ -z "${platform}" ]; then
    echo ""
    return 0
  fi
  platform="${platform#linux/}"
  platform="${platform//\//-}"
  if [ -z "${platform}" ] || [ "${platform}" = "linux" ]; then
    echo ""
    return 0
  fi
  echo "-${platform}"
}

build_bundle::maybe_run_linux_build_in_docker() {
  # With `set -e`, this function must return 0 when no Docker cross-build is needed.
  if [ "${PLATFORM}" != "linux" ] || [ "$(uname -s)" = "Linux" ] || [ -n "${BUNDLE_IN_CONTAINER:-}" ]; then
    return 0
  fi

  command -v docker >/dev/null 2>&1 || build_bundle::fail "Docker is required to build a Linux bundle from non-Linux host"
  [ -n "${DOCKER_PLATFORM}" ] || build_bundle::fail "--docker-platform must not be empty"

  local node_path_env="${NOMOS_NODE_PATH}"
  local -a extra_mounts=()
  if [ -n "${NOMOS_NODE_PATH}" ]; then
    case "${NOMOS_NODE_PATH}" in
      "${ROOT_DIR}"/*)
        node_path_env="/workspace${NOMOS_NODE_PATH#"${ROOT_DIR}"}"
        ;;
      /*)
        node_path_env="/external/logos-blockchain-node"
        extra_mounts+=("-v" "${NOMOS_NODE_PATH}:${node_path_env}")
        ;;
      *)
        build_bundle::fail "--path must be absolute when cross-building in Docker"
        ;;
    esac
  fi

  echo "==> Building Linux bundle inside Docker"
  local container_output="/workspace${OUTPUT#"${ROOT_DIR}"}"
  local target_suffix
  target_suffix="$(build_bundle::docker_platform_suffix "${DOCKER_PLATFORM}")"
  local host_target_dir="${ROOT_DIR}/.tmp/logos-blockchain-node-linux-target${target_suffix}"
  mkdir -p "${ROOT_DIR}/.tmp/cargo-linux" "${host_target_dir}"

  local -a features_args=()
  if [ -n "${NOMOS_EXTRA_FEATURES:-}" ]; then
    features_args+=(--features "${NOMOS_EXTRA_FEATURES}")
  fi

  local -a src_args=()
  if [ -n "${node_path_env}" ]; then
    src_args+=(--path "${node_path_env}")
  else
    src_args+=(--rev "${NOMOS_NODE_REV}")
  fi

  docker run --rm --platform "${DOCKER_PLATFORM}" \
    -e VERSION="${VERSION}" \
    -e NOMOS_NODE_REV="${NOMOS_NODE_REV}" \
    -e NOMOS_NODE_PATH="${node_path_env}" \
    -e NOMOS_BUNDLE_DOCKER_PLATFORM="${DOCKER_PLATFORM}" \
    -e NOMOS_EXTRA_FEATURES="${NOMOS_EXTRA_FEATURES:-}" \
    -e BUNDLE_IN_CONTAINER=1 \
    -e CARGO_HOME=/workspace/.tmp/cargo-linux \
    -e CARGO_TARGET_DIR="/workspace/.tmp/logos-blockchain-node-linux-target${target_suffix}" \
    -v "${ROOT_DIR}/.tmp/cargo-linux":/workspace/.tmp/cargo-linux \
    -v "${host_target_dir}:/workspace/.tmp/logos-blockchain-node-linux-target${target_suffix}" \
    -v "${ROOT_DIR}:/workspace" \
    "${extra_mounts[@]}" \
    -w /workspace \
    "${DOCKER_RUST_IMAGE}" \
    bash -c "apt-get update && apt-get install -y ${DOCKER_APT_PACKAGES[*]} && ./scripts/build/build-bundle.sh --platform linux --output \"${container_output}\" ${src_args[*]} ${features_args[*]}"

  exit 0
}

build_bundle::prepare_circuits() {
  echo "==> Preparing build workspace (version ${VERSION})"
  if [ "${PLATFORM}" = "host" ]; then
    NODE_TARGET="${ROOT_DIR}/.tmp/logos-blockchain-node-host-target"
  else
    # When building Linux bundles in Docker, avoid reusing the same target dir
    # across different container architectures (e.g. linux/arm64 vs linux/amd64),
    # as the native-host `target/debug` layout would otherwise get mixed.
    local target_suffix=""
    if [ -n "${BUNDLE_IN_CONTAINER:-}" ]; then
      target_suffix="$(build_bundle::docker_platform_suffix "${NOMOS_BUNDLE_DOCKER_PLATFORM:-}")"
    fi
    NODE_TARGET="${ROOT_DIR}/.tmp/logos-blockchain-node-linux-target${target_suffix}"
  fi

  NODE_SRC_DEFAULT="${ROOT_DIR}/.tmp/logos-blockchain-node-${PLATFORM}-src"
  NODE_SRC="${NOMOS_NODE_PATH:-${NODE_SRC_DEFAULT}}"
  if [ -n "${NOMOS_NODE_PATH}" ]; then
    [ -d "${NODE_SRC}" ] || build_bundle::fail "NOMOS_NODE_PATH does not exist: ${NODE_SRC}"
    rm -rf "${NODE_SRC_DEFAULT}"
    if [ -d "${NODE_TARGET}" ]; then
      find "${NODE_TARGET}" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
    fi
    NODE_TARGET="${NODE_TARGET}-local"
  fi

  NODE_BIN="${NODE_TARGET}/debug/logos-blockchain-node"
}

build_bundle::build_binaries() {
  BUILD_FEATURES_LABEL="all"
  echo "==> Building binaries (platform=${PLATFORM})"
  mkdir -p "${NODE_SRC}"
  (
    cd "${NODE_SRC}"
    if [ -n "${NOMOS_NODE_PATH}" ]; then
      echo "Using local logos-blockchain-node checkout at ${NODE_SRC} (no fetch/checkout)"
    else
      if [ ! -d "${NODE_SRC}/.git" ]; then
        git clone https://github.com/logos-co/nomos-node.git "${NODE_SRC}"
      fi
      git fetch --depth 1 origin "${NOMOS_NODE_REV}"
      git checkout "${NOMOS_NODE_REV}"
      git reset --hard
      git clean -fdx
    fi

    if [ -z "${NOMOS_NODE_PATH}" ]; then
      build_bundle::apply_nomos_node_patches "${NODE_SRC}"
    fi
    unset CARGO_FEATURE_BUILD_VERIFICATION_KEY
    if [ -n "${BUNDLE_RUSTUP_TOOLCHAIN}" ]; then
      RUSTFLAGS='--cfg feature="pol-dev-mode"' \
        RUSTUP_TOOLCHAIN="${BUNDLE_RUSTUP_TOOLCHAIN}" \
        cargo build --all-features \
        -p logos-blockchain-node \
        --target-dir "${NODE_TARGET}"
    else
      RUSTFLAGS='--cfg feature="pol-dev-mode"' \
        cargo build --all-features \
        -p logos-blockchain-node \
        --target-dir "${NODE_TARGET}"
    fi
  )
}

build_bundle::package_bundle() {
  echo "==> Packaging bundle"
  local bundle_dir="${ROOT_DIR}/.tmp/nomos-bundle"
  rm -rf "${bundle_dir}"
  mkdir -p "${bundle_dir}/artifacts"
  cp "${NODE_BIN}" "${bundle_dir}/artifacts/logos-blockchain-node"
  {
    echo "nomos_node_path=${NOMOS_NODE_PATH:-}"
    echo "nomos_node_rev=${NOMOS_NODE_REV:-}"
    if [ -d "${NODE_SRC}/.git" ] && command -v git >/dev/null 2>&1; then
      echo "nomos_node_git_head=$(git -C "${NODE_SRC}" rev-parse HEAD 2>/dev/null || true)"
    fi
    echo "platform=${PLATFORM}"
    echo "features=${BUILD_FEATURES_LABEL}"
  } > "${bundle_dir}/artifacts/nomos-bundle-meta.env"

  mkdir -p "$(dirname "${OUTPUT}")"
  if tar --help 2>/dev/null | grep -q -- '--no-mac-metadata'; then
    tar --no-mac-metadata --no-xattrs -czf "${OUTPUT}" -C "${bundle_dir}" artifacts
  elif tar --help 2>/dev/null | grep -q -- '--no-xattrs'; then
    tar --no-xattrs -czf "${OUTPUT}" -C "${bundle_dir}" artifacts
  else
    tar -czf "${OUTPUT}" -C "${bundle_dir}" artifacts
  fi
  echo "Bundle created at ${OUTPUT}"

  if [[ "${BUILD_FEATURES_LABEL}" == "all" ]] || [[ "${BUILD_FEATURES_LABEL}" == *profiling* ]]; then
    cat <<'EOF_PROF'
Profiling endpoints (enabled by --features profiling):
  CPU pprof (SVG):   curl "http://<node-host>:8722/debug/pprof/profile?seconds=15&format=svg" -o profile.svg
  CPU pprof (proto): go tool pprof -http=:8080 "http://<node-host>:8722/debug/pprof/profile?seconds=15&format=proto"
EOF_PROF
  fi
}

build_bundle::main() {
  build_bundle::load_env
  build_bundle::clean_cargo_linux_cache
  build_bundle::parse_args "$@"
  build_bundle::validate_and_finalize
  build_bundle::maybe_run_linux_build_in_docker
  build_bundle::prepare_circuits
  build_bundle::build_binaries
  build_bundle::package_bundle
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  build_bundle::main "$@"
fi
