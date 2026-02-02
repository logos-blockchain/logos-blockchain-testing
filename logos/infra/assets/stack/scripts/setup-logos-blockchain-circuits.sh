#!/bin/bash
#
# Setup script for logos-blockchain-circuits
#
# Usage: ./setup-logos-blockchain-circuits.sh [VERSION] [INSTALL_DIR]
#   VERSION      - Optional. Version to install (default: v0.3.1)
#   INSTALL_DIR  - Optional. Installation directory (default: $HOME/.logos-blockchain-circuits)
#
# Examples:
#   ./setup-logos-blockchain-circuits.sh                    # Install default version to default location
#   ./setup-logos-blockchain-circuits.sh v0.2.0             # Install specific version to default location
#   ./setup-logos-blockchain-circuits.sh v0.2.0 /opt/circuits  # Install to custom location

set -euo pipefail

readonly DEFAULT_CIRCUITS_VERSION="v0.4.1"
readonly DEFAULT_INSTALL_SUBDIR=".logos-blockchain-circuits"
readonly DEFAULT_CIRCUITS_REPO="logos-blockchain/logos-blockchain-circuits"

readonly CURL_RETRY_COUNT=5
readonly CURL_RETRY_DELAY_SECONDS=2

VERSION="${1:-${DEFAULT_CIRCUITS_VERSION}}"
DEFAULT_INSTALL_DIR="${HOME}/${DEFAULT_INSTALL_SUBDIR}"
INSTALL_DIR="${2:-${DEFAULT_INSTALL_DIR}}"
REPO="${DEFAULT_CIRCUITS_REPO}"

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
    local artifact=""
    local url=""
    local temp_dir
    temp_dir=$(mktemp -d)

    for attempt in 1 2; do
        artifact="logos-blockchain-circuits-${VERSION}-${platform}.tar.gz"
        url="https://github.com/${REPO}/releases/download/${VERSION}/${artifact}"

        echo "Downloading logos-blockchain-circuits ${VERSION} for ${platform}..."
        local -a curl_args=(
          curl
          -fL
          --retry "${CURL_RETRY_COUNT}"
          --retry-delay "${CURL_RETRY_DELAY_SECONDS}"
        )
        # `curl` is not guaranteed to support `--retry-all-errors`, so check before using it
        # `curl --help` may be abbreviated on some platforms
        if (curl --help all 2>/dev/null || curl --help 2>/dev/null) | grep -q -- '--retry-all-errors'; then
          curl_args+=(--retry-all-errors)
        fi

        if [ -n "${GITHUB_TOKEN:-}" ]; then
            curl_args+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
        fi
        curl_args+=(-o "${temp_dir}/${artifact}" "${url}")

        if "${curl_args[@]}"; then
            if tar -tzf "${temp_dir}/${artifact}" >/dev/null 2>&1; then
                break
            fi
            if [ "${platform}" = "linux-aarch64" ] || [ "${platform}" = "linux-arm64" ]; then
                echo "Downloaded artifact is not a valid tar.gz; falling back to linux-x86_64" >&2
                rm -f "${temp_dir}/${artifact}"
                platform="linux-x86_64"
                continue
            fi
            echo "Downloaded artifact is not a valid tar.gz for ${platform}" >&2
            rm -rf "${temp_dir}"
            exit 1
        fi

        if [ "${attempt}" -eq 1 ] && { [ "${platform}" = "linux-aarch64" ] || [ "${platform}" = "linux-arm64" ]; }; then
            echo "No linux-aarch64 assets found; falling back to linux-x86_64" >&2
            platform="linux-x86_64"
            continue
        fi

        echo "Failed to download release artifact from ${url}" >&2
        rm -rf "${temp_dir}"
        exit 1
    done

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

platform="${LOGOS_BLOCKCHAIN_CIRCUITS_PLATFORM:-$(detect_platform)}"
echo "Setting up logos-blockchain-circuits ${VERSION} for ${platform}"
echo "Installing to ${INSTALL_DIR}"

download_release "${platform}"

echo "Installation complete. Circuits installed at: ${INSTALL_DIR}"
echo "If using a custom directory, set LOGOS_BLOCKCHAIN_CIRCUITS=${INSTALL_DIR}"
