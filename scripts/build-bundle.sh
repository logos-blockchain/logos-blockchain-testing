#!/usr/bin/env bash
set -euo pipefail

# Build a nomos-binaries.tar.gz for the specified platform.
#
# Usage: scripts/build-bundle.sh [--platform host|linux] [--output PATH] [--rev REV | --path DIR] [--features LIST] [--docker-platform PLATFORM]
#   --platform        Target platform for binaries (default: host)
#   --output          Output path for the tarball (default: .tmp/nomos-binaries-<platform>-<version>.tar.gz)
#   --rev             nomos-node git revision to build (overrides NOMOS_NODE_REV)
#   --path            Use local nomos-node checkout at DIR (skip fetch/checkout)
#   --features        Extra cargo features to enable (comma-separated); base always includes "testing"
#   --docker-platform Docker platform for Linux bundle when running on non-Linux host (default: auto; linux/arm64 on Apple silicon Docker Desktop, else linux/amd64)

# Always run under bash; bail out if someone invokes via sh.
if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

usage() {
  cat <<'USAGE'
Usage: scripts/build-bundle.sh [--platform host|linux] [--output PATH]

Options:
  --platform        Target platform for binaries (default: host)
  --output          Output path for the tarball (default: .tmp/nomos-binaries-<platform>-<version>.tar.gz)
  --rev             nomos-node git revision to build (overrides NOMOS_NODE_REV)
  --path            Use local nomos-node checkout at DIR (skip fetch/checkout)
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

fail() { echo "$1" >&2; exit 1; }

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

NOMOS_EXTRA_FEATURES="${NOMOS_EXTRA_FEATURES:-}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -f "${ROOT_DIR}/versions.env" ]; then
  # shellcheck disable=SC1091
  . "${ROOT_DIR}/versions.env"
else
  echo "ERROR: versions.env missing; run from repo root or restore the file." >&2
  exit 1
fi
DEFAULT_VERSION="${VERSION:?Missing VERSION in versions.env}"
DEFAULT_NODE_REV="${NOMOS_NODE_REV:-}"
DEFAULT_NODE_PATH="${NOMOS_NODE_PATH:-}"
PLATFORM="host"
OUTPUT=""
REV_OVERRIDE=""
PATH_OVERRIDE=""
DOCKER_PLATFORM="${NOMOS_BUNDLE_DOCKER_PLATFORM:-${NOMOS_BIN_PLATFORM:-}}"
BUNDLE_RUSTUP_TOOLCHAIN="${BUNDLE_RUSTUP_TOOLCHAIN:-}"

if [ -z "${BUNDLE_RUSTUP_TOOLCHAIN}" ] && command -v rustup >/dev/null 2>&1 && [ -f "${ROOT_DIR}/rust-toolchain.toml" ]; then
  BUNDLE_RUSTUP_TOOLCHAIN="$(awk -F '\"' '/^[[:space:]]*channel[[:space:]]*=/{print $2; exit}' "${ROOT_DIR}/rust-toolchain.toml")"
fi

# Default Docker platform to the engine architecture when possible.
if [ -z "${DOCKER_PLATFORM}" ] && command -v docker >/dev/null 2>&1; then
  docker_arch="$(docker version --format '{{.Server.Arch}}' 2>/dev/null || true)"
  case "${docker_arch}" in
    arm64|aarch64) DOCKER_PLATFORM="linux/arm64" ;;
    amd64|x86_64) DOCKER_PLATFORM="linux/amd64" ;;
    *) DOCKER_PLATFORM="linux/amd64" ;;
  esac
fi
DOCKER_PLATFORM="${DOCKER_PLATFORM:-linux/amd64}"

# To avoid confusing cache corruption errors inside the Dockerized Linux build,
# always start from a clean cargo registry/git cache for the cross-build.
rm -rf "${ROOT_DIR}/.tmp/cargo-linux/registry" "${ROOT_DIR}/.tmp/cargo-linux/git"

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
    *) fail "Unknown argument: $1" ;;
  esac
done

case "$PLATFORM" in
  host|linux) ;;
  *) fail "--platform must be host or linux" ;;
esac

VERSION="${DEFAULT_VERSION}"
if [ -n "${REV_OVERRIDE}" ] && [ -n "${PATH_OVERRIDE}" ]; then
  fail "Use either --rev or --path, not both"
fi
if [ -z "${REV_OVERRIDE}" ] && [ -z "${PATH_OVERRIDE}" ] && [ -z "${DEFAULT_NODE_REV}" ] && [ -z "${DEFAULT_NODE_PATH}" ]; then
  fail "Provide --rev, --path, or set NOMOS_NODE_REV/NOMOS_NODE_PATH in versions.env"
fi
NOMOS_NODE_REV="${REV_OVERRIDE:-${DEFAULT_NODE_REV}}"
NOMOS_NODE_PATH="${PATH_OVERRIDE:-${DEFAULT_NODE_PATH}}"

# Normalize OUTPUT to an absolute path under the workspace.
if [ -z "${OUTPUT}" ]; then
  OUTPUT="${ROOT_DIR}/.tmp/nomos-binaries-${PLATFORM}-${VERSION}.tar.gz"
elif [[ "${OUTPUT}" != /* ]]; then
  OUTPUT="${ROOT_DIR}/${OUTPUT#./}"
fi
echo "Bundle output: ${OUTPUT}"

if [ "$PLATFORM" = "linux" ] && [ "$(uname -s)" != "Linux" ] && [ -z "${BUNDLE_IN_CONTAINER:-}" ]; then
  if ! command -v docker >/dev/null 2>&1; then
    fail "Docker is required to build a Linux bundle from non-Linux host"
  fi
  if [ -z "${DOCKER_PLATFORM}" ]; then
    fail "--docker-platform must not be empty"
  fi

  NODE_PATH_ENV="${NOMOS_NODE_PATH}"
  EXTRA_MOUNTS=()
  if [ -n "${NOMOS_NODE_PATH}" ]; then
    case "${NOMOS_NODE_PATH}" in
      "${ROOT_DIR}"/*)
        NODE_PATH_ENV="/workspace${NOMOS_NODE_PATH#"${ROOT_DIR}"}"
        ;;
      /*)
        NODE_PATH_ENV="/external/nomos-node"
        EXTRA_MOUNTS+=("-v" "${NOMOS_NODE_PATH}:${NODE_PATH_ENV}")
        ;;
      *)
        fail "--path must be absolute when cross-building in Docker"
        ;;
    esac
  fi

  echo "==> Building Linux bundle inside Docker"
  container_output="/workspace${OUTPUT#"${ROOT_DIR}"}"
  mkdir -p "${ROOT_DIR}/.tmp/cargo-linux" "${ROOT_DIR}/.tmp/nomos-node-linux-target"

  FEATURES_ARGS=()
  if [ -n "${NOMOS_EXTRA_FEATURES:-}" ]; then
    FEATURES_ARGS+=(--features "${NOMOS_EXTRA_FEATURES}")
  fi

  SRC_ARGS=()
  if [ -n "${NODE_PATH_ENV}" ]; then
    SRC_ARGS+=(--path "${NODE_PATH_ENV}")
  else
    SRC_ARGS+=(--rev "${NOMOS_NODE_REV}")
  fi

  docker run --rm --platform "${DOCKER_PLATFORM}" \
    -e VERSION="$VERSION" \
    -e NOMOS_NODE_REV="$NOMOS_NODE_REV" \
    -e NOMOS_NODE_PATH="$NODE_PATH_ENV" \
    -e NOMOS_CIRCUITS="/workspace/.tmp/nomos-circuits-linux" \
    -e STACK_DIR="/workspace/.tmp/nomos-circuits-linux" \
    -e HOST_DIR="/workspace/.tmp/nomos-circuits-linux" \
    -e NOMOS_EXTRA_FEATURES="${NOMOS_EXTRA_FEATURES:-}" \
    -e BUNDLE_IN_CONTAINER=1 \
    -e CARGO_HOME=/workspace/.tmp/cargo-linux \
    -e CARGO_TARGET_DIR=/workspace/.tmp/nomos-node-linux-target \
    -v "${ROOT_DIR}/.tmp/cargo-linux":/workspace/.tmp/cargo-linux \
    -v "${ROOT_DIR}/.tmp/nomos-node-linux-target":/workspace/.tmp/nomos-node-linux-target \
    -v "$ROOT_DIR":/workspace \
    "${EXTRA_MOUNTS[@]}" \
    -w /workspace \
    rust:1.80-bullseye \
    bash -c "apt-get update && apt-get install -y clang llvm-dev libclang-dev pkg-config cmake libssl-dev rsync libgmp10 libgmp-dev libgomp1 nasm && ./scripts/build-bundle.sh --platform linux --output \"${container_output}\" ${SRC_ARGS[*]} ${FEATURES_ARGS[*]}"

  exit 0
fi

echo "==> Preparing circuits (version ${VERSION})"
if [ "$PLATFORM" = "host" ]; then
  CIRCUITS_DIR="${ROOT_DIR}/.tmp/nomos-circuits-host"
  NODE_TARGET="${ROOT_DIR}/.tmp/nomos-node-host-target"
else
  CIRCUITS_DIR="${ROOT_DIR}/.tmp/nomos-circuits-linux"
  NODE_TARGET="${ROOT_DIR}/.tmp/nomos-node-linux-target"
fi

NODE_SRC_DEFAULT="${ROOT_DIR}/.tmp/nomos-node-${PLATFORM}-src"
NODE_SRC="${NOMOS_NODE_PATH:-${NODE_SRC_DEFAULT}}"
if [ -n "${NOMOS_NODE_PATH}" ]; then
  if [ ! -d "${NODE_SRC}" ]; then
    fail "NOMOS_NODE_PATH does not exist: ${NODE_SRC}"
  fi
  rm -rf "${NODE_SRC_DEFAULT}"
  if [ -d "${NODE_TARGET}" ]; then
    find "${NODE_TARGET}" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  fi
  NODE_TARGET="${NODE_TARGET}-local"
fi

export NOMOS_CIRCUITS="${CIRCUITS_DIR}"
mkdir -p "${ROOT_DIR}/.tmp" "${CIRCUITS_DIR}"
if [ -f "${CIRCUITS_DIR}/${KZG_FILE:-kzgrs_test_params}" ]; then
  echo "Circuits already present at ${CIRCUITS_DIR}; skipping download"
else
  STACK_DIR="${CIRCUITS_DIR}" HOST_DIR="${CIRCUITS_DIR}" \
    "${ROOT_DIR}/scripts/setup-circuits-stack.sh" "${VERSION}" </dev/null
fi

NODE_BIN="${NODE_TARGET}/debug/nomos-node"
EXEC_BIN="${NODE_TARGET}/debug/nomos-executor"
CLI_BIN="${NODE_TARGET}/debug/nomos-cli"

FEATURES="testing"
if [ -n "${NOMOS_EXTRA_FEATURES:-}" ]; then
  FEATURES="${FEATURES},${NOMOS_EXTRA_FEATURES}"
fi

echo "==> Building binaries (platform=${PLATFORM})"
mkdir -p "${NODE_SRC}"
(
  cd "${NODE_SRC}"
  if [ -n "${NOMOS_NODE_PATH}" ]; then
    echo "Using local nomos-node checkout at ${NODE_SRC} (no fetch/checkout)"
  else
    if [ ! -d "${NODE_SRC}/.git" ]; then
      git clone https://github.com/logos-co/nomos-node.git "${NODE_SRC}"
    fi
    git fetch --depth 1 origin "${NOMOS_NODE_REV}"
    git checkout "${NOMOS_NODE_REV}"
    git reset --hard
    git clean -fdx
  fi
  if [ -n "${BUNDLE_RUSTUP_TOOLCHAIN}" ]; then
    RUSTFLAGS='--cfg feature="pol-dev-mode"' NOMOS_CIRCUITS="${CIRCUITS_DIR}" \
      RUSTUP_TOOLCHAIN="${BUNDLE_RUSTUP_TOOLCHAIN}" \
      cargo build --features "${FEATURES}" \
      -p nomos-node -p nomos-executor -p nomos-cli \
      --target-dir "${NODE_TARGET}"
  else
    RUSTFLAGS='--cfg feature="pol-dev-mode"' NOMOS_CIRCUITS="${CIRCUITS_DIR}" \
      cargo build --features "${FEATURES}" \
      -p nomos-node -p nomos-executor -p nomos-cli \
      --target-dir "${NODE_TARGET}"
  fi
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
{
  echo "nomos_node_path=${NOMOS_NODE_PATH:-}"
  echo "nomos_node_rev=${NOMOS_NODE_REV:-}"
  if [ -d "${NODE_SRC}/.git" ] && command -v git >/dev/null 2>&1; then
    echo "nomos_node_git_head=$(git -C "${NODE_SRC}" rev-parse HEAD 2>/dev/null || true)"
  fi
  echo "platform=${PLATFORM}"
  echo "features=${FEATURES}"
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

if [[ "${FEATURES}" == *profiling* ]]; then
  cat <<'EOF_PROF'
Profiling endpoints (enabled by --features profiling):
  CPU pprof (SVG):   curl "http://<node-host>:8722/debug/pprof/profile?seconds=15&format=svg" -o profile.svg
  CPU pprof (proto): go tool pprof -http=:8080 "http://<node-host>:8722/debug/pprof/profile?seconds=15&format=proto"
EOF_PROF
fi
