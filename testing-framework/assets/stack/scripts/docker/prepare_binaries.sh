#!/usr/bin/env bash
set -euo pipefail

LOGOS_BLOCKCHAIN_NODE_REV="${LOGOS_BLOCKCHAIN_NODE_REV:?LOGOS_BLOCKCHAIN_NODE_REV build arg missing}"

mkdir -p /workspace/artifacts

TARGET_ARCH="$(uname -m)"

have_prebuilt() {
  [ -f testing-framework/assets/stack/bin/logos-blockchain-node ] && \
  [ -f testing-framework/assets/stack/bin/logos-blockchain-node ]
}

bin_matches_arch() {
  local info
  info="$(file -b testing-framework/assets/stack/bin/logos-blockchain-node 2>/dev/null || true)"
  case "${info}" in
    *ELF*) : ;;
    *) return 1 ;;
  esac

  local pattern
  case "${TARGET_ARCH}" in
    x86_64) pattern="x86-64|x86_64" ;;
    aarch64|arm64) pattern="arm64|aarch64" ;;
    *) pattern="${TARGET_ARCH}" ;;
  esac

  echo "${info}" | grep -Eqi "${pattern}"
}

if have_prebuilt && bin_matches_arch; then
  echo "Using prebuilt logos-blockchain binaries from testing-framework/assets/stack/bin"
  cp testing-framework/assets/stack/bin/logos-blockchain-node /workspace/artifacts/logos-blockchain-node
  exit 0
fi

if have_prebuilt; then
  echo "Prebuilt logos-blockchain binaries do not match target architecture (${TARGET_ARCH}); rebuilding from source"
else
  echo "Prebuilt logos-blockchain binaries missing; building from source"
fi

echo "Building logos-blockchain binaries from source (rev ${LOGOS_BLOCKCHAIN_NODE_REV})"
git clone https://github.com/logos-co/nomos-node.git /tmp/nomos-node
cd /tmp/nomos-node
git fetch --depth 1 origin "${LOGOS_BLOCKCHAIN_NODE_REV}"
git checkout "${LOGOS_BLOCKCHAIN_NODE_REV}"
git reset --hard
git clean -fdx

# Enable pol-dev-mode via cfg to let POL_PROOF_DEV_MODE short-circuit proofs in tests.
RUSTFLAGS='--cfg feature="pol-dev-mode"' \
  cargo build --features "testing" -p logos-blockchain-node

cp /tmp/nomos-node/target/debug/logos-blockchain-node /workspace/artifacts/logos-blockchain-node

rm -rf /tmp/nomos-node/target/debug/incremental
