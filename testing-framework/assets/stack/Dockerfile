# syntax=docker/dockerfile:1
# check=skip=SecretsUsedInArgOrEnv
# Ignore warnings about sensitive information as this is test data.

ARG VERSION=v0.3.1
ARG CIRCUITS_OVERRIDE

# ===========================
# BUILD IMAGE
# ===========================

FROM rust:1.91.0-slim-bookworm AS builder

ARG VERSION
ARG CIRCUITS_OVERRIDE

LABEL maintainer="augustinas@status.im" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos testnet build image"

WORKDIR /workspace
COPY . .

# Reduce debug artifact size.
ENV CARGO_PROFILE_DEV_DEBUG=0

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
ENV CARGO_TARGET_DIR=/workspace/target

# Fetch the nomos-node sources pinned in Cargo.lock and build the runtime binaries.
RUN if [ ! -d /workspace/nomos-node ]; then \
      git clone https://github.com/logos-co/nomos-node.git /workspace/nomos-node; \
    fi && \
    cd /workspace/nomos-node && \
    git fetch --depth 1 origin 2f60a0372c228968c3526c341ebc7e58bbd178dd && \
    git checkout 2f60a0372c228968c3526c341ebc7e58bbd178dd && \
    git reset --hard && git clean -fdx && \
    cargo build --all-features --bins && \
    rm -rf /workspace/nomos-node/target/debug/incremental

# Build cfgsync binaries from this workspace.
RUN cargo build --all-features --manifest-path /workspace/testing-framework/tools/cfgsync/Cargo.toml --bins

# ===========================
# NODE IMAGE
# ===========================

FROM debian:bookworm-slim

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

COPY --from=builder /workspace/target/debug/nomos-node /usr/bin/nomos-node
COPY --from=builder /workspace/target/debug/nomos-executor /usr/bin/nomos-executor
COPY --from=builder /workspace/target/debug/nomos-cli /usr/bin/nomos-cli
COPY --from=builder /workspace/target/debug/cfgsync-server /usr/bin/cfgsync-server
COPY --from=builder /workspace/target/debug/cfgsync-client /usr/bin/cfgsync-client

ENV NOMOS_CIRCUITS=/opt/circuits

EXPOSE 3000 8080 9000 60000

ENTRYPOINT ["/usr/bin/nomos-node"]
