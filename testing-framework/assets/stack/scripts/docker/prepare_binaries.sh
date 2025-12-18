#!/usr/bin/env bash
set -euo pipefail

NOMOS_NODE_REV="${NOMOS_NODE_REV:?NOMOS_NODE_REV build arg missing}"

mkdir -p /workspace/artifacts

TARGET_ARCH="$(uname -m)"

have_prebuilt() {
  [ -f testing-framework/assets/stack/bin/nomos-node ] && \
  [ -f testing-framework/assets/stack/bin/nomos-executor ] && \
  [ -f testing-framework/assets/stack/bin/nomos-cli ]
}

bin_matches_arch() {
  local info
  info="$(file -b testing-framework/assets/stack/bin/nomos-node 2>/dev/null || true)"
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
  echo "Using prebuilt nomos binaries from testing-framework/assets/stack/bin"
  cp testing-framework/assets/stack/bin/nomos-node /workspace/artifacts/nomos-node
  cp testing-framework/assets/stack/bin/nomos-executor /workspace/artifacts/nomos-executor
  cp testing-framework/assets/stack/bin/nomos-cli /workspace/artifacts/nomos-cli
  exit 0
fi

if have_prebuilt; then
  echo "Prebuilt nomos binaries do not match target architecture (${TARGET_ARCH}); rebuilding from source"
else
  echo "Prebuilt nomos binaries missing; building from source"
fi

echo "Building nomos binaries from source (rev ${NOMOS_NODE_REV})"
git clone https://github.com/logos-co/nomos-node.git /tmp/nomos-node
cd /tmp/nomos-node
git fetch --depth 1 origin "${NOMOS_NODE_REV}"
git checkout "${NOMOS_NODE_REV}"
git reset --hard
git clean -fdx

# Enable pol-dev-mode via cfg to let POL_PROOF_DEV_MODE short-circuit proofs in tests.
RUSTFLAGS='--cfg feature="pol-dev-mode"' NOMOS_CIRCUITS=/opt/circuits cargo build --features "testing" \
  -p nomos-node -p nomos-executor -p nomos-cli

cp /tmp/nomos-node/target/debug/nomos-node /workspace/artifacts/nomos-node
cp /tmp/nomos-node/target/debug/nomos-executor /workspace/artifacts/nomos-executor
cp /tmp/nomos-node/target/debug/nomos-cli /workspace/artifacts/nomos-cli

rm -rf /tmp/nomos-node/target/debug/incremental

