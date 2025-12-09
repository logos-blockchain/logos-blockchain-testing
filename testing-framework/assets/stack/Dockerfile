# syntax=docker/dockerfile:1
# check=skip=SecretsUsedInArgOrEnv
# Ignore warnings about sensitive information as this is test data.

ARG VERSION=v0.3.1
ARG CIRCUITS_OVERRIDE
ARG NOMOS_NODE_REV=d2dd5a5084e1daef4032562c77d41de5e4d495f8

# ===========================
# BUILD IMAGE
# ===========================

FROM rust:1.91.0-slim-bookworm AS builder

ARG VERSION
ARG CIRCUITS_OVERRIDE
ARG NOMOS_NODE_REV

LABEL maintainer="augustinas@status.im" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos testnet build image"

WORKDIR /workspace
COPY . .

# Reduce debug artifact size.
ENV CARGO_PROFILE_DEV_DEBUG=0
ENV NOMOS_NODE_REV=${NOMOS_NODE_REV}

# Install dependencies needed for building RocksDB.
RUN apt-get update && apt-get install -yq \
    git gcc g++ clang make cmake m4 xz-utils libgmp-dev libssl-dev pkg-config ca-certificates curl wget file

RUN mkdir -p /opt/circuits && \
    select_circuits_source() { \
        # Prefer an explicit override when it exists (file or directory). \
        if [ -n "$CIRCUITS_OVERRIDE" ] && [ -e "/workspace/${CIRCUITS_OVERRIDE}" ]; then \
            echo "/workspace/${CIRCUITS_OVERRIDE}"; \
            return 0; \
        fi; \
        # Fall back to the workspace bundle shipped with the repo. \
        if [ -e "/workspace/tests/kzgrs/kzgrs_test_params" ]; then \
            echo "/workspace/tests/kzgrs/kzgrs_test_params"; \
            return 0; \
        fi; \
        return 1; \
    }; \
    if CIRCUITS_PATH="$(select_circuits_source)"; then \
        echo "Using prebuilt circuits bundle from ${CIRCUITS_PATH#/workspace/}"; \
        if [ -d "$CIRCUITS_PATH" ]; then \
            cp -R "${CIRCUITS_PATH}/." /opt/circuits; \
        else \
            cp "${CIRCUITS_PATH}" /opt/circuits/; \
        fi; \
    fi; \
    TARGET_ARCH="$(uname -m)"; \
    if [ -f "/opt/circuits/prover" ]; then \
        PROVER_INFO="$(file -b /opt/circuits/prover || true)"; \
        case "$TARGET_ARCH" in \
            x86_64) EXPECT_ARCH="x86-64" ;; \
            aarch64|arm64) EXPECT_ARCH="aarch64" ;; \
            *) EXPECT_ARCH="$TARGET_ARCH" ;; \
        esac; \
        if [ -n "$PROVER_INFO" ] && ! echo "$PROVER_INFO" | grep -qi "$EXPECT_ARCH"; then \
            echo "Circuits prover architecture ($PROVER_INFO) does not match target ${TARGET_ARCH}; rebuilding rapidsnark binaries"; \
            chmod +x scripts/build-rapidsnark.sh && \
            RAPIDSNARK_FORCE_REBUILD=1 \
                scripts/build-rapidsnark.sh /opt/circuits; \
        fi; \
    fi; \
    if [ ! -f "/opt/circuits/pol/verification_key.json" ]; then \
        echo "Local circuits missing pol artifacts; downloading ${VERSION} bundle and rebuilding"; \
        chmod +x scripts/setup-nomos-circuits.sh && \
        NOMOS_CIRCUITS_REBUILD_RAPIDSNARK=1 \
        RAPIDSNARK_BUILD_GMP=1 \
            scripts/setup-nomos-circuits.sh "$VERSION" "/opt/circuits"; \
    fi

ENV NOMOS_CIRCUITS=/opt/circuits

# Provide runtime binaries. Prefer prebuilt artifacts (when present) for speed; otherwise build from source.
RUN set -eu; \
    mkdir -p /workspace/artifacts; \
    TARGET_ARCH="$(uname -m)"; \
    expect_arch() { \
      case "$1" in \
        x86_64) echo "x86-64" ;; \
        aarch64|arm64) echo "arm64" ;; \
        *) echo "$1" ;; \
      esac; \
    }; \
    have_prebuilt() { \
      [ -f testing-framework/assets/stack/bin/nomos-node ] && \
      [ -f testing-framework/assets/stack/bin/nomos-executor ] && \
      [ -f testing-framework/assets/stack/bin/nomos-cli ]; \
    }; \
    bin_matches_arch() { \
      BIN_INFO="$(file -b testing-framework/assets/stack/bin/nomos-node 2>/dev/null || true)"; \
      case "$BIN_INFO" in \
        *ELF*);; \
        *) return 1 ;; \
      esac; \
      case "$TARGET_ARCH" in \
        x86_64) PATTERN="x86-64|x86_64" ;; \
        aarch64|arm64) PATTERN="arm64|aarch64" ;; \
        *) PATTERN="$(expect_arch "$TARGET_ARCH")" ;; \
      esac; \
      [ -n "$BIN_INFO" ] && echo "$BIN_INFO" | grep -Eqi "$PATTERN"; \
    }; \
    if have_prebuilt; then \
      if bin_matches_arch; then \
        echo "Using prebuilt nomos binaries from testing-framework/assets/stack/bin"; \
      else \
        echo "Using prebuilt nomos binaries from testing-framework/assets/stack/bin (arch check skipped)"; \
      fi; \
      cp testing-framework/assets/stack/bin/nomos-node /workspace/artifacts/nomos-node; \
      cp testing-framework/assets/stack/bin/nomos-executor /workspace/artifacts/nomos-executor; \
      cp testing-framework/assets/stack/bin/nomos-cli /workspace/artifacts/nomos-cli; \
    else \
      echo "Prebuilt nomos binaries missing or wrong architecture; building from source (rev ${NOMOS_NODE_REV})"; \
      git clone https://github.com/logos-co/nomos-node.git /tmp/nomos-node && \
      cd /tmp/nomos-node && \
      git fetch --depth 1 origin "${NOMOS_NODE_REV}" && \
      git checkout "${NOMOS_NODE_REV}" && \
      git reset --hard && git clean -fdx && \
      # Enable pol-dev-mode via cfg to let POL_PROOF_DEV_MODE short-circuit proofs in tests.
      RUSTFLAGS='--cfg feature="pol-dev-mode"' NOMOS_CIRCUITS=/opt/circuits cargo build --features "testing" \
        -p nomos-node -p nomos-executor -p nomos-cli; \
      cp /tmp/nomos-node/target/debug/nomos-node /workspace/artifacts/nomos-node; \
      cp /tmp/nomos-node/target/debug/nomos-executor /workspace/artifacts/nomos-executor; \
      cp /tmp/nomos-node/target/debug/nomos-cli /workspace/artifacts/nomos-cli; \
      rm -rf /tmp/nomos-node/target/debug/incremental; \
    fi

# Build cfgsync binaries from this workspace.
RUN cargo build --all-features --manifest-path /workspace/testing-framework/tools/cfgsync/Cargo.toml --bins
RUN cp /workspace/target/debug/cfgsync-server /workspace/artifacts/cfgsync-server && \
    cp /workspace/target/debug/cfgsync-client /workspace/artifacts/cfgsync-client && \
    rm -rf /workspace/target/debug/incremental

# ===========================
# NODE IMAGE
# ===========================

FROM ubuntu:24.04

ARG VERSION

LABEL maintainer="augustinas@status.im" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos node image"

RUN apt-get update && apt-get install -yq \
    libstdc++6 \
    libgmp10 \
    libgomp1 \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/circuits /opt/circuits

COPY --from=builder /workspace/artifacts/nomos-node /usr/bin/nomos-node
COPY --from=builder /workspace/artifacts/nomos-executor /usr/bin/nomos-executor
COPY --from=builder /workspace/artifacts/nomos-cli /usr/bin/nomos-cli
COPY --from=builder /workspace/artifacts/cfgsync-server /usr/bin/cfgsync-server
COPY --from=builder /workspace/artifacts/cfgsync-client /usr/bin/cfgsync-client

ENV NOMOS_CIRCUITS=/opt/circuits

EXPOSE 3000 8080 9000 60000

ENTRYPOINT ["/usr/bin/nomos-node"]
