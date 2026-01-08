#!/usr/bin/env bash
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

readonly DEFAULT_CIRCUITS_VERSION="v0.3.1"
readonly DEFAULT_INSTALL_SUBDIR=".nomos-circuits"
readonly DEFAULT_CIRCUITS_REPO="logos-co/nomos-circuits"

readonly DEFAULT_NONINTERACTIVE=0
readonly DEFAULT_REBUILD_RAPIDSNARK=0

readonly CURL_RETRY_COUNT=5
readonly CURL_RETRY_DELAY_SECONDS=2

readonly ANSI_RED=$'\033[0;31m'
readonly ANSI_GREEN=$'\033[0;32m'
readonly ANSI_YELLOW=$'\033[1;33m'
readonly ANSI_BLUE=$'\033[0;34m'
readonly ANSI_RESET=$'\033[0m'

readonly ICON_INFO="ℹ"
readonly ICON_OK="✓"
readonly ICON_WARN="⚠"
readonly ICON_ERR="✗"

setup_nomos_circuits::usage() {
  cat <<EOF
Usage: scripts/setup/setup-nomos-circuits.sh [VERSION] [INSTALL_DIR]

Arguments:
  VERSION      Optional. Version to install (default: ${DEFAULT_CIRCUITS_VERSION})
  INSTALL_DIR  Optional. Installation directory (default: \$HOME/${DEFAULT_INSTALL_SUBDIR})

Environment:
  NOMOS_CIRCUITS_PLATFORM            Override platform (e.g. linux-x86_64, macos-aarch64)
  NOMOS_CIRCUITS_NONINTERACTIVE      Set to 1 to auto-overwrite without prompt
  NOMOS_CIRCUITS_REBUILD_RAPIDSNARK  Set to 1 to force rapidsnark rebuild
  GITHUB_TOKEN                       Optional token for GitHub releases download
EOF
}

setup_nomos_circuits::init_vars() {
  VERSION="${1:-${DEFAULT_CIRCUITS_VERSION}}"
  DEFAULT_INSTALL_DIR="${HOME}/${DEFAULT_INSTALL_SUBDIR}"
  INSTALL_DIR="${2:-${DEFAULT_INSTALL_DIR}}"
  REPO="${DEFAULT_CIRCUITS_REPO}"
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  NONINTERACTIVE="${NOMOS_CIRCUITS_NONINTERACTIVE:-${DEFAULT_NONINTERACTIVE}}"

  # Colors for output
  RED="${ANSI_RED}"
  GREEN="${ANSI_GREEN}"
  YELLOW="${ANSI_YELLOW}"
  BLUE="${ANSI_BLUE}"
  NC="${ANSI_RESET}"
}

setup_nomos_circuits::print_info() { echo -e "${BLUE}${ICON_INFO}${NC} $1"; }
setup_nomos_circuits::print_success() { echo -e "${GREEN}${ICON_OK}${NC} $1"; }
setup_nomos_circuits::print_warning() { echo -e "${YELLOW}${ICON_WARN}${NC} $1"; }
setup_nomos_circuits::print_error() { echo -e "${RED}${ICON_ERR}${NC} $1"; }

setup_nomos_circuits::detect_platform() {
  local os="" arch=""

  case "$(uname -s)" in
    Linux*) os="linux" ;;
    Darwin*) os="macos" ;;
    MINGW*|MSYS*|CYGWIN*) os="windows" ;;
    *) setup_nomos_circuits::print_error "Unsupported operating system: $(uname -s)"; exit 1 ;;
  esac

  case "$(uname -m)" in
    x86_64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) setup_nomos_circuits::print_error "Unsupported architecture: $(uname -m)"; exit 1 ;;
  esac

  echo "${os}-${arch}"
}

setup_nomos_circuits::check_existing_installation() {
  if [ -d "${INSTALL_DIR}" ]; then
    setup_nomos_circuits::print_warning "Installation directory already exists: ${INSTALL_DIR}"

    if [ -f "${INSTALL_DIR}/VERSION" ]; then
      local current_version
      current_version="$(cat "${INSTALL_DIR}/VERSION")"
      setup_nomos_circuits::print_info "Currently installed version: ${current_version}"
    fi

    if [ "${NONINTERACTIVE}" = "1" ] || [ ! -t 0 ]; then
      setup_nomos_circuits::print_info "Non-interactive environment detected, automatically overwriting..."
    else
      echo
      read -p "Do you want to overwrite it? (y/N): " -n 1 -r
      echo
      if [[ ! ${REPLY} =~ ^[Yy]$ ]]; then
        setup_nomos_circuits::print_info "Installation cancelled."
        exit 0
      fi
    fi

    setup_nomos_circuits::print_info "Removing existing installation..."
    rm -rf "${INSTALL_DIR}"
  fi
}

setup_nomos_circuits::download_release() {
  local platform="$1"
  local artifact="nomos-circuits-${VERSION}-${platform}.tar.gz"
  local url="https://github.com/${REPO}/releases/download/${VERSION}/${artifact}"
  local temp_dir
  temp_dir="$(mktemp -d)"

  setup_nomos_circuits::print_info "Downloading nomos-circuits ${VERSION} for ${platform}..."
  setup_nomos_circuits::print_info "URL: ${url}"

  local -a curl_args=(
    curl
    -fL
    --retry "${CURL_RETRY_COUNT}"
    --retry-delay "${CURL_RETRY_DELAY_SECONDS}"
  )
  if curl --help 2>/dev/null | grep -q -- '--retry-all-errors'; then
    curl_args+=(--retry-all-errors)
  fi

  if [ -n "${GITHUB_TOKEN:-}" ]; then
    curl_args+=(--header "authorization: Bearer ${GITHUB_TOKEN}")
  fi
  curl_args+=(-o "${temp_dir}/${artifact}" "${url}")

  if ! "${curl_args[@]}"; then
    setup_nomos_circuits::print_error "Failed to download release artifact"
    setup_nomos_circuits::print_error "Please check that version ${VERSION} exists for platform ${platform}"
    setup_nomos_circuits::print_error "Available releases: https://github.com/${REPO}/releases"
    rm -rf "${temp_dir}"
    return 1
  fi

  setup_nomos_circuits::print_success "Download complete"

  if ! tar -tzf "${temp_dir}/${artifact}" >/dev/null 2>&1; then
    setup_nomos_circuits::print_error "Downloaded archive is not a valid tar.gz: ${temp_dir}/${artifact}"
    rm -rf "${temp_dir}"
    return 1
  fi

  setup_nomos_circuits::print_info "Extracting to ${INSTALL_DIR}..."
  mkdir -p "${INSTALL_DIR}"

  if ! tar -xzf "${temp_dir}/${artifact}" -C "${INSTALL_DIR}" --strip-components=1; then
    setup_nomos_circuits::print_error "Failed to extract archive"
    rm -rf "${temp_dir}"
    return 1
  fi

  rm -rf "${temp_dir}"
  setup_nomos_circuits::print_success "Extraction complete"
}

setup_nomos_circuits::handle_macos_quarantine() {
  setup_nomos_circuits::print_info "macOS detected: Removing quarantine attributes from executables..."

  if find "${INSTALL_DIR}" -type f -perm -111 -exec xattr -d com.apple.quarantine {} \; 2>/dev/null; then
    setup_nomos_circuits::print_success "Quarantine attributes removed"
  else
    setup_nomos_circuits::print_warning "Could not remove quarantine attributes (they may not exist)"
  fi
}

setup_nomos_circuits::print_circuits() {
  setup_nomos_circuits::print_info "The following circuits are available:"
  local dir circuit_name
  for dir in "${INSTALL_DIR}"/*/; do
    if [ -d "${dir}" ]; then
      circuit_name="$(basename "${dir}")"
      if [ -f "${dir}/witness_generator" ]; then
        echo "  • ${circuit_name}"
      fi
    fi
  done
}

setup_nomos_circuits::resolve_platform() {
  local platform_override="${NOMOS_CIRCUITS_PLATFORM:-}"
  if [ -n "${platform_override}" ]; then
    PLATFORM="${platform_override}"
    setup_nomos_circuits::print_info "Using overridden platform: ${PLATFORM}"
  else
    PLATFORM="$(setup_nomos_circuits::detect_platform)"
    setup_nomos_circuits::print_info "Detected platform: ${PLATFORM}"
  fi
}

setup_nomos_circuits::download_with_fallbacks() {
  # Outputs:
  #   PLATFORM - platform used for the downloaded bundle
  #   REBUILD_REQUIRED - 0/1
  REBUILD_REQUIRED="${NOMOS_CIRCUITS_REBUILD_RAPIDSNARK:-${DEFAULT_REBUILD_RAPIDSNARK}}"

  if setup_nomos_circuits::download_release "${PLATFORM}"; then
    return 0
  fi

  if [[ "${PLATFORM}" == "linux-aarch64" ]]; then
    setup_nomos_circuits::print_warning "Falling back to linux-x86_64 circuits bundle; will rebuild prover for aarch64."
    rm -rf "${INSTALL_DIR}"
    PLATFORM="linux-x86_64"
    setup_nomos_circuits::download_release "${PLATFORM}" || return 1
    REBUILD_REQUIRED=1
    return 0
  fi

  if [[ "${PLATFORM}" == "macos-x86_64" ]]; then
    setup_nomos_circuits::print_warning "No macOS x86_64 bundle; falling back to macOS aarch64 circuits bundle and rebuilding prover."
    rm -rf "${INSTALL_DIR}"
    PLATFORM="macos-aarch64"
    if ! setup_nomos_circuits::download_release "${PLATFORM}"; then
      setup_nomos_circuits::print_warning "macOS aarch64 bundle unavailable; trying linux-x86_64 bundle and rebuilding prover."
      rm -rf "${INSTALL_DIR}"
      PLATFORM="linux-x86_64"
      setup_nomos_circuits::download_release "${PLATFORM}" || return 1
    fi
    REBUILD_REQUIRED=1
    return 0
  fi

  return 1
}

setup_nomos_circuits::maybe_handle_quarantine() {
  if [[ "${PLATFORM}" == macos-* ]]; then
    echo
    setup_nomos_circuits::handle_macos_quarantine
  fi
}

setup_nomos_circuits::maybe_rebuild_rapidsnark() {
  if [[ "${REBUILD_REQUIRED}" == "1" ]]; then
    echo
    setup_nomos_circuits::print_info "Rebuilding rapidsnark prover for ${PLATFORM}..."
    "${SCRIPT_DIR}/build/build-rapidsnark.sh" "${INSTALL_DIR}"
  else
    setup_nomos_circuits::print_info "Skipping rapidsnark rebuild (set NOMOS_CIRCUITS_REBUILD_RAPIDSNARK=1 to force)."
  fi
}

setup_nomos_circuits::print_summary() {
  echo
  setup_nomos_circuits::print_success "Installation complete!"
  echo
  setup_nomos_circuits::print_info "nomos-circuits ${VERSION} is now installed at: ${INSTALL_DIR}"
  setup_nomos_circuits::print_circuits

  if [ "${INSTALL_DIR}" != "${DEFAULT_INSTALL_DIR}" ]; then
    echo
    setup_nomos_circuits::print_info "Since you're using a custom installation directory, set the environment variable:"
    setup_nomos_circuits::print_info "  export NOMOS_CIRCUITS=${INSTALL_DIR}"
    echo
  fi
}

setup_nomos_circuits::main() {
  if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
    setup_nomos_circuits::usage
    exit 0
  fi

  setup_nomos_circuits::init_vars "${1:-}" "${2:-}"

  setup_nomos_circuits::print_info "Setting up nomos-circuits ${VERSION}"
  setup_nomos_circuits::print_info "Installation directory: ${INSTALL_DIR}"
  echo

  setup_nomos_circuits::resolve_platform

  setup_nomos_circuits::check_existing_installation

  setup_nomos_circuits::download_with_fallbacks || exit 1
  setup_nomos_circuits::maybe_handle_quarantine
  setup_nomos_circuits::maybe_rebuild_rapidsnark
  setup_nomos_circuits::print_summary
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  setup_nomos_circuits::main "$@"
fi
