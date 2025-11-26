#!/bin/bash
#
# Setup script for nomos-circuits
#
# Usage: ./setup-nomos-circuits.sh [VERSION] [INSTALL_DIR]
#   VERSION      - Optional. Version to install (default: v0.3.1)
#   INSTALL_DIR  - Optional. Installation directory (default: $HOME/.nomos-circuits)
#
# Examples:
#   ./setup-nomos-circuits.sh                    # Install default version to default location
#   ./setup-nomos-circuits.sh v0.2.0             # Install specific version to default location
#   ./setup-nomos-circuits.sh v0.2.0 /opt/circuits  # Install to custom location

set -euo pipefail

VERSION="${1:-v0.3.1}"
DEFAULT_INSTALL_DIR="$HOME/.nomos-circuits"
INSTALL_DIR="${2:-$DEFAULT_INSTALL_DIR}"
REPO="logos-co/nomos-circuits"

detect_platform() {
    local os=""
    local arch=""
    case "$(uname -s)" in
        Linux*) os="linux" ;;
        Darwin*) os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) echo "Unsupported operating system: $(uname -s)" >&2; exit 1 ;;
    esac
    case "$(uname -m)" in
        x86_64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
    esac
    echo "${os}-${arch}"
}

download_release() {
    local platform="$1"
    local artifact="nomos-circuits-${VERSION}-${platform}.tar.gz"
    local url="https://github.com/${REPO}/releases/download/${VERSION}/${artifact}"
    local temp_dir
    temp_dir=$(mktemp -d)

    echo "Downloading nomos-circuits ${VERSION} for ${platform}..."
    if [ -n "${GITHUB_TOKEN:-}" ]; then
        auth_header="Authorization: Bearer ${GITHUB_TOKEN}"
    else
        auth_header=""
    fi

    if ! curl -L ${auth_header:+-H "$auth_header"} -o "${temp_dir}/${artifact}" "${url}"; then
        echo "Failed to download release artifact from ${url}" >&2
        rm -rf "${temp_dir}"
        exit 1
    fi

    echo "Extracting to ${INSTALL_DIR}..."
    rm -rf "${INSTALL_DIR}"
    mkdir -p "${INSTALL_DIR}"
    if ! tar -xzf "${temp_dir}/${artifact}" -C "${INSTALL_DIR}" --strip-components=1; then
        echo "Failed to extract ${artifact}" >&2
        rm -rf "${temp_dir}"
        exit 1
    fi
    rm -rf "${temp_dir}"
}

platform=$(detect_platform)
echo "Setting up nomos-circuits ${VERSION} for ${platform}"
echo "Installing to ${INSTALL_DIR}"

download_release "${platform}"

echo "Installation complete. Circuits installed at: ${INSTALL_DIR}"
echo "If using a custom directory, set NOMOS_CIRCUITS=${INSTALL_DIR}"
