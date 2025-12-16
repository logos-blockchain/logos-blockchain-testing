#!/bin/bash
#
# Rebuild the rapidsnark prover for the current architecture.
#
# Usage: ./scripts/build-rapidsnark.sh <circuits_dir>

set -euo pipefail

readonly DEFAULT_RAPIDSNARK_REPO="https://github.com/iden3/rapidsnark.git"
readonly DEFAULT_RAPIDSNARK_REF="main"
readonly DEFAULT_FORCE_REBUILD=0
readonly DEFAULT_USE_ASM="ON"

if [ $# -lt 1 ]; then
    echo "usage: $0 <circuits_dir>" >&2
    exit 1
fi

TARGET_ARCH="$(uname -m)"
CIRCUITS_DIR="$1"
RAPIDSNARK_REPO="${RAPIDSNARK_REPO:-${DEFAULT_RAPIDSNARK_REPO}}"
RAPIDSNARK_REF="${RAPIDSNARK_REF:-${DEFAULT_RAPIDSNARK_REF}}"
FORCE_REBUILD="${RAPIDSNARK_FORCE_REBUILD:-${DEFAULT_FORCE_REBUILD}}"
BUILD_DIR=""
PACKAGE_DIR=""
CMAKE_TARGET_PLATFORM=""
USE_ASM="${RAPIDSNARK_USE_ASM:-${DEFAULT_USE_ASM}}"
CMAKE_C_FLAGS="${RAPIDSNARK_C_FLAGS:-}"
CMAKE_CXX_FLAGS="${RAPIDSNARK_CXX_FLAGS:-}"

if [ ! -d "$CIRCUITS_DIR" ]; then
    echo "circuits directory '$CIRCUITS_DIR' does not exist" >&2
    exit 1
fi

system_gmp_package() {
    local multiarch arch
    arch="${1:-${TARGET_ARCH}}"
    multiarch="$(gcc -print-multiarch 2>/dev/null || echo "${arch}-linux-gnu")"
    local lib_path="/usr/lib/${multiarch}/libgmp.a"
    if [ ! -f "$lib_path" ]; then
        echo "system libgmp.a not found at $lib_path" >&2
        return 1
    fi
    mkdir -p "depends/gmp/package_${arch}/lib" "depends/gmp/package_${arch}/include"
    cp "$lib_path" "depends/gmp/package_${arch}/lib/"
    # Headers are small; copy the public ones the build expects.
    cp /usr/include/gmp*.h "depends/gmp/package_${arch}/include/" || true
}

case "$TARGET_ARCH" in
    arm64 | aarch64)
        CMAKE_TARGET_PLATFORM="aarch64"
        BUILD_DIR="build_prover_arm64"
        PACKAGE_DIR="${RAPIDSNARK_PACKAGE_DIR:-package_arm64}"
        ;;
    x86_64)
        if [ "$FORCE_REBUILD" != "1" ]; then
            echo "rapidsnark rebuild skipped for architecture '$TARGET_ARCH' (set RAPIDSNARK_FORCE_REBUILD=1 to override)" >&2
            exit 0
        fi
        if [ -z "$CMAKE_C_FLAGS" ]; then
            # Keep CPU requirements minimal so binaries run under emulation (e.g. act on Apple hosts).
            CMAKE_C_FLAGS="-march=x86-64 -mno-avx -mno-avx2 -mno-sse4.2"
        fi
        if [ -z "$CMAKE_CXX_FLAGS" ]; then
            CMAKE_CXX_FLAGS="$CMAKE_C_FLAGS"
        fi
        # Assembly paths assume modern CPU features; disable by default for x86_64 unless overridden.
        if [ "${RAPIDSNARK_USE_ASM:-}" = "" ]; then
            USE_ASM="OFF"
        fi
        CMAKE_TARGET_PLATFORM="x86_64"
        BUILD_DIR="build_prover_x86_64"
        PACKAGE_DIR="${RAPIDSNARK_PACKAGE_DIR:-package_x86_64}"
        ;;
    *)
        if [ "$FORCE_REBUILD" != "1" ]; then
            echo "rapidsnark rebuild skipped for unsupported architecture '$TARGET_ARCH'" >&2
            exit 0
        fi
        CMAKE_TARGET_PLATFORM="$TARGET_ARCH"
        BUILD_DIR="build_prover_${TARGET_ARCH}"
        PACKAGE_DIR="${RAPIDSNARK_PACKAGE_DIR:-package_${TARGET_ARCH}}"
        ;;
esac

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo "Building rapidsnark ($RAPIDSNARK_REF) for $TARGET_ARCH..." >&2
git clone --depth 1 --branch "$RAPIDSNARK_REF" "$RAPIDSNARK_REPO" "$workdir/rapidsnark" >&2
cd "$workdir/rapidsnark"
git submodule update --init --recursive >&2

if [ "${RAPIDSNARK_BUILD_GMP:-1}" = "1" ]; then
    if [ -z "${RAPIDSNARK_GMP_TARGET:-}" ]; then
        if [ "$CMAKE_TARGET_PLATFORM" = "x86_64" ]; then
            GMP_TARGET="host"
        else
            GMP_TARGET="$CMAKE_TARGET_PLATFORM"
        fi
    else
        GMP_TARGET="$RAPIDSNARK_GMP_TARGET"
    fi
    ./build_gmp.sh "$GMP_TARGET" >&2
else
    echo "Using system libgmp to satisfy rapidsnark dependencies" >&2
    system_gmp_package "$CMAKE_TARGET_PLATFORM"
fi

rm -rf "$BUILD_DIR"
mkdir "$BUILD_DIR"
cd "$BUILD_DIR"
cmake .. \
    -DTARGET_PLATFORM="$CMAKE_TARGET_PLATFORM" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="../${PACKAGE_DIR}" \
    -DBUILD_SHARED_LIBS=OFF \
    -DUSE_ASM="$USE_ASM" \
    ${CMAKE_C_FLAGS:+-DCMAKE_C_FLAGS="$CMAKE_C_FLAGS"} \
    ${CMAKE_CXX_FLAGS:+-DCMAKE_CXX_FLAGS="$CMAKE_CXX_FLAGS"} >&2
cmake --build . --target prover verifier -- -j"$(nproc)" >&2

install -m 0755 "src/prover" "$CIRCUITS_DIR/prover"
install -m 0755 "src/verifier" "$CIRCUITS_DIR/verifier"
echo "rapidsnark prover installed to $CIRCUITS_DIR/prover" >&2
