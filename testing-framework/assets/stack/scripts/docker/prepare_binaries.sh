#!/usr/bin/env bash
set -euo pipefail

NOMOS_NODE_REV="${NOMOS_NODE_REV:?NOMOS_NODE_REV build arg missing}"

mkdir -p /workspace/artifacts

TARGET_ARCH="$(uname -m)"

have_prebuilt() {
  [ -f testing-framework/assets/stack/bin/logos-blockchain-node ] && \
  [ -f testing-framework/assets/stack/bin/logos-blockchain-cli ]
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
  cp testing-framework/assets/stack/bin/logos-blockchain-cli /workspace/artifacts/logos-blockchain-cli
  exit 0
fi

if have_prebuilt; then
  echo "Prebuilt logos-blockchain binaries do not match target architecture (${TARGET_ARCH}); rebuilding from source"
else
  echo "Prebuilt logos-blockchain binaries missing; building from source"
fi

echo "Building logos-blockchain binaries from source (rev ${NOMOS_NODE_REV})"
git clone https://github.com/logos-co/nomos-node.git /tmp/nomos-node
cd /tmp/nomos-node
git fetch --depth 1 origin "${NOMOS_NODE_REV}"
git checkout "${NOMOS_NODE_REV}"
git reset --hard
git clean -fdx

# Enable real verification keys when available.
if [ -f "/opt/circuits/zksign/verification_key.json" ] \
  || [ -f "/opt/circuits/pol/verification_key.json" ] \
  || [ -f "/opt/circuits/poq/verification_key.json" ] \
  || [ -f "/opt/circuits/poc/verification_key.json" ]; then
  export CARGO_FEATURE_BUILD_VERIFICATION_KEY=1
else
  unset CARGO_FEATURE_BUILD_VERIFICATION_KEY
fi

# Enable pol-dev-mode via cfg to let POL_PROOF_DEV_MODE short-circuit proofs in tests.
RUSTFLAGS='--cfg feature="pol-dev-mode"' NOMOS_CIRCUITS=/opt/circuits \
  LOGOS_BLOCKCHAIN_CIRCUITS=/opt/circuits \
  cargo build --features "testing" \
  -p logos-blockchain-node -p logos-blockchain-cli

cp /tmp/nomos-node/target/debug/logos-blockchain-node /workspace/artifacts/logos-blockchain-node
cp /tmp/nomos-node/target/debug/logos-blockchain-cli /workspace/artifacts/logos-blockchain-cli

rm -rf /tmp/nomos-node/target/debug/incremental
