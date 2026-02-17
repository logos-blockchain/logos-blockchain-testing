#!/usr/bin/env bash
set -euo pipefail

LOGOS_BLOCKCHAIN_NODE_REV="${LOGOS_BLOCKCHAIN_NODE_REV:?LOGOS_BLOCKCHAIN_NODE_REV build arg missing}"

mkdir -p /workspace/artifacts

TARGET_ARCH="$(uname -m)"

have_prebuilt() {
  [ -f logos/infra/assets/stack/bin/logos-blockchain-node ] && \
  [ -f logos/infra/assets/stack/bin/logos-blockchain-node ]
}

bin_matches_arch() {
  local info
  info="$(file -b logos/infra/assets/stack/bin/logos-blockchain-node 2>/dev/null || true)"
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

if [ -n "${LOGOS_BLOCKCHAIN_FORCE_BUILD:-}" ]; then
  echo "LOGOS_BLOCKCHAIN_FORCE_BUILD is set; rebuilding logos-blockchain binaries from source"
elif have_prebuilt && bin_matches_arch; then
  echo "Using prebuilt logos-blockchain binaries from logos/infra/assets/stack/bin"
  cp logos/infra/assets/stack/bin/logos-blockchain-node /workspace/artifacts/logos-blockchain-node
  exit 0
fi

if have_prebuilt; then
  echo "Prebuilt logos-blockchain binaries do not match target architecture (${TARGET_ARCH}); rebuilding from source"
else
  echo "Prebuilt logos-blockchain binaries missing; building from source"
fi

echo "Building logos-blockchain binaries from source (rev ${LOGOS_BLOCKCHAIN_NODE_REV})"
if [ "${LOGOS_BLOCKCHAIN_NODE_USE_LOCAL_CONTEXT:-0}" = "1" ] && [ -d /nomos-node ]; then
  echo "Using local nomos-node checkout from Docker build context"
  cd /nomos-node
else
  git clone https://github.com/logos-co/nomos-node.git /tmp/nomos-node
  cd /tmp/nomos-node
  git fetch --depth 1 origin "${LOGOS_BLOCKCHAIN_NODE_REV}"
  git checkout "${LOGOS_BLOCKCHAIN_NODE_REV}"
  git reset --hard
  git clean -fdx
fi

# Enable pol-dev-mode and embed verification keys for proof validation.
RUSTFLAGS='--cfg feature="pol-dev-mode" --cfg feature="build-verification-key"' \
  CARGO_FEATURE_BUILD_VERIFICATION_KEY=1 \
  cargo build --all-features -p logos-blockchain-node

cp target/debug/logos-blockchain-node /workspace/artifacts/logos-blockchain-node

rm -rf target/debug/incremental
