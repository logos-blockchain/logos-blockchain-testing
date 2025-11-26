#!/bin/bash
#
# Rebuild the rapidsnark prover for the current architecture.
#
# Usage: ./scripts/build-rapidsnark.sh <circuits_dir>

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "usage: $0 <circuits_dir>" >&2
    exit 1
fi

TARGET_ARCH="$(uname -m)"
CIRCUITS_DIR="$1"
RAPIDSNARK_REPO="${RAPIDSNARK_REPO:-https://github.com/iden3/rapidsnark.git}"
RAPIDSNARK_REF="${RAPIDSNARK_REF:-main}"

if [ ! -d "$CIRCUITS_DIR" ]; then
    echo "circuits directory '$CIRCUITS_DIR' does not exist" >&2
    exit 1
fi

system_gmp_package() {
    local multiarch
    multiarch="$(gcc -print-multiarch 2>/dev/null || echo aarch64-linux-gnu)"
    local lib_path="/usr/lib/${multiarch}/libgmp.a"
    if [ ! -f "$lib_path" ]; then
        echo "system libgmp.a not found at $lib_path" >&2
        return 1
    fi
    mkdir -p depends/gmp/package_aarch64/lib depends/gmp/package_aarch64/include
    cp "$lib_path" depends/gmp/package_aarch64/lib/
    # Headers are small; copy the public ones the build expects.
    cp /usr/include/gmp*.h depends/gmp/package_aarch64/include/ || true
}

case "$TARGET_ARCH" in
    arm64 | aarch64)
        ;;
    *)
        echo "rapidsnark rebuild skipped for architecture '$TARGET_ARCH'" >&2
        exit 0
        ;;
esac

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo "Building rapidsnark ($RAPIDSNARK_REF) for $TARGET_ARCH..." >&2
git clone --depth 1 --branch "$RAPIDSNARK_REF" "$RAPIDSNARK_REPO" "$workdir/rapidsnark" >&2
cd "$workdir/rapidsnark"
git submodule update --init --recursive >&2

if [ "${RAPIDSNARK_BUILD_GMP:-1}" = "1" ]; then
    GMP_TARGET="${RAPIDSNARK_GMP_TARGET:-aarch64}"
    ./build_gmp.sh "$GMP_TARGET" >&2
else
    echo "Using system libgmp to satisfy rapidsnark dependencies" >&2
    system_gmp_package
fi

PACKAGE_DIR="${RAPIDSNARK_PACKAGE_DIR:-package_arm64}"

rm -rf build_prover_arm64
mkdir build_prover_arm64
cd build_prover_arm64
cmake .. \
    -DTARGET_PLATFORM=aarch64 \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="../${PACKAGE_DIR}" \
    -DBUILD_SHARED_LIBS=OFF >&2
cmake --build . --target prover verifier -- -j"$(nproc)" >&2

install -m 0755 "src/prover" "$CIRCUITS_DIR/prover"
install -m 0755 "src/verifier" "$CIRCUITS_DIR/verifier"
echo "rapidsnark prover installed to $CIRCUITS_DIR/prover" >&2
