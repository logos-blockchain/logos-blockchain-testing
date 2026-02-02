#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

# Setup script for logos-blockchain-circuits
#
# Usage: scripts/setup/setup-logos-blockchain-circuits.sh [VERSION] [INSTALL_DIR]
#
# Arguments:
#   VERSION      Optional. Version to install (default: v0.3.2)
#   INSTALL_DIR  Optional. Installation directory (default: $HOME/.logos-blockchain-circuits)
#
# Examples:
#   scripts/setup/setup-logos-blockchain-circuits.sh
#   scripts/setup/setup-logos-blockchain-circuits.sh v0.3.2
#   scripts/setup/setup-logos-blockchain-circuits.sh v0.3.2 /opt/circuits

DEFAULT_CIRCUITS_VERSION="v0.4.1"
DEFAULT_INSTALL_DIR="${HOME}/.logos-blockchain-circuits"
REPO="logos-blockchain/logos-blockchain-circuits"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_info() { echo -e "${BLUE}ℹ${NC} $1"; }
print_success() { echo -e "${GREEN}✓${NC} $1"; }
print_warning() { echo -e "${YELLOW}⚠${NC} $1"; }
print_error() { echo -e "${RED}✗${NC} $1"; }

VERSION="${1:-${DEFAULT_CIRCUITS_VERSION}}"
INSTALL_DIR="${2:-${DEFAULT_INSTALL_DIR}}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RAPIDSNARK_SKIP="${RAPIDSNARK_SKIP:-0}"

# Detect OS and architecture
# Outputs: os-arch like linux-x86_64, macos-aarch64
#
# Uses same logic as the logos-blockchain-node installer.
detect_platform() {
  local os="" arch=""

  case "$(uname -s)" in
    Linux*) os="linux" ;;
    Darwin*) os="macos" ;;
    MINGW*|MSYS*|CYGWIN*) os="windows" ;;
    *) print_error "Unsupported operating system: $(uname -s)"; exit 1 ;;
  esac

  case "$(uname -m)" in
    x86_64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) print_error "Unsupported architecture: $(uname -m)"; exit 1 ;;
  esac

  echo "${os}-${arch}"
}

check_existing_installation() {
  if [ -d "${INSTALL_DIR}" ]; then
    print_warning "Installation directory already exists: ${INSTALL_DIR}"

    if [ -f "${INSTALL_DIR}/VERSION" ]; then
      local current_version
      current_version="$(cat "${INSTALL_DIR}/VERSION")"
      print_info "Currently installed version: ${current_version}"
    fi

    if [ ! -t 0 ]; then
      print_info "Non-interactive environment detected, automatically overwriting..."
    else
      echo
      read -p "Do you want to overwrite it? (y/N): " -n 1 -r
      echo
      if [[ ! ${REPLY} =~ ^[Yy]$ ]]; then
        print_info "Installation cancelled."
        exit 0
      fi
    fi

    print_info "Removing existing installation..."
    rm -rf "${INSTALL_DIR}"
  fi
}

download_release() {
  local platform="$1"
  local artifact=""
  local url=""
  local temp_dir
  temp_dir="$(mktemp -d)"

  for attempt in 1 2; do
    artifact="logos-blockchain-circuits-${VERSION}-${platform}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${VERSION}/${artifact}"

    print_info "Downloading logos-blockchain-circuits ${VERSION} for ${platform}..."
    print_info "URL: ${url}"

    local curl_cmd="curl -L"
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      curl_cmd="$curl_cmd --header 'authorization: Bearer ${GITHUB_TOKEN}'"
    fi
    curl_cmd="$curl_cmd -o ${temp_dir}/${artifact} ${url}"

    if eval "${curl_cmd}"; then
      if tar -tzf "${temp_dir}/${artifact}" >/dev/null 2>&1; then
        break
      fi
      if [ "${platform}" = "linux-aarch64" ] || [ "${platform}" = "linux-arm64" ]; then
        print_warning "Downloaded artifact is not a valid tar.gz; falling back to linux-x86_64"
        rm -f "${temp_dir}/${artifact}"
        platform="linux-x86_64"
        continue
      fi
      print_error "Downloaded artifact is not a valid tar.gz for ${platform}"
      rm -rf "${temp_dir}"
      exit 1
    fi

    if [ "${attempt}" -eq 1 ] && { [ "${platform}" = "linux-aarch64" ] || [ "${platform}" = "linux-arm64" ]; }; then
      print_warning "No linux-aarch64 assets found; falling back to linux-x86_64"
      platform="linux-x86_64"
      continue
    fi

    print_error "Failed to download release artifact"
    print_error "Please check that version ${VERSION} exists for platform ${platform}"
    print_error "Available releases: https://github.com/${REPO}/releases"
    rm -rf "${temp_dir}"
    exit 1
  done

  print_success "Download complete"

  print_info "Extracting to ${INSTALL_DIR}..."
  mkdir -p "${INSTALL_DIR}"

  if ! tar -xzf "${temp_dir}/${artifact}" -C "${INSTALL_DIR}" --strip-components=1; then
    print_error "Failed to extract archive"
    rm -rf "${temp_dir}"
    exit 1
  fi

  rm -rf "${temp_dir}"
  print_success "Extraction complete"
}

handle_macos_quarantine() {
  print_info "macOS detected: Removing quarantine attributes from executables..."

  if find "${INSTALL_DIR}" -type f -perm +111 -exec xattr -d com.apple.quarantine {} \; 2>/dev/null; then
    print_success "Quarantine attributes removed"
  else
    print_warning "Could not remove quarantine attributes (they may not exist)"
  fi
}

print_circuits() {
  print_info "The following circuits are available:"

  local dir
  for dir in "${INSTALL_DIR}"/*/; do
    if [ -d "${dir}" ] && [ -f "${dir}/witness_generator" ]; then
      echo "  • $(basename "${dir}")"
    fi
  done
}

ensure_rapidsnark() {
  if [ -x "${INSTALL_DIR}/prover" ] && [ -x "${INSTALL_DIR}/verifier" ]; then
    print_info "rapidsnark prover already present at ${INSTALL_DIR}"
    return
  fi

  if [ "${RAPIDSNARK_SKIP}" = "1" ]; then
    print_warning "Skipping rapidsnark build (RAPIDSNARK_SKIP=1). Proofs may fail without prover/verifier."
    return
  fi

  local build_script="${REPO_ROOT}/scripts/build/build-rapidsnark.sh"
  if [ ! -x "${build_script}" ]; then
    print_error "rapidsnark build script not found or not executable: ${build_script}"
    exit 1
  fi

  local missing=()
  for tool in git cmake make gcc g++; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
      missing+=("${tool}")
    fi
  done

  if [ "${#missing[@]}" -gt 0 ]; then
    print_error "Missing build tools for rapidsnark: ${missing[*]}"
    print_error "Install the required packages, or set RAPIDSNARK_SKIP=1 to skip the build."
    exit 1
  fi

  print_info "Building rapidsnark prover into ${INSTALL_DIR}..."
  bash "${build_script}" "${INSTALL_DIR}"
  print_success "rapidsnark prover installed"
}

main() {
  print_info "Setting up logos-blockchain-circuits ${VERSION}"
  print_info "Installation directory: ${INSTALL_DIR}"
  echo

  local platform
  platform="$(detect_platform)"
  print_info "Detected platform: ${platform}"

  check_existing_installation
  download_release "${platform}"

  if [[ "${platform}" == macos-* ]]; then
    echo
    handle_macos_quarantine
  fi

  echo
  ensure_rapidsnark

  echo
  print_success "Installation complete!"
  echo
  print_info "logos-blockchain-circuits ${VERSION} is now installed at: ${INSTALL_DIR}"
  print_circuits

  if [ "${INSTALL_DIR}" != "${DEFAULT_INSTALL_DIR}" ]; then
    echo
    print_info "Since you're using a custom installation directory, set the environment variable:"
    print_info "  export LOGOS_BLOCKCHAIN_CIRCUITS=${INSTALL_DIR}"
    echo
  fi
}

main "$@"
