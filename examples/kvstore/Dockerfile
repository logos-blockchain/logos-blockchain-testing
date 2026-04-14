# Build stage
FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

# Copy all workspace files required for workspace build.
COPY Cargo.toml Cargo.lock ./
COPY cfgsync/ ./cfgsync/
COPY examples/ ./examples/
COPY testing-framework/ ./testing-framework/

# Build kvstore-node in release mode.
RUN cargo build --release -p kvstore-node

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/kvstore-node /usr/local/bin/kvstore-node

RUN mkdir -p /etc/kvstore
WORKDIR /app

ENTRYPOINT ["/usr/local/bin/kvstore-node"]
CMD ["--config", "/etc/kvstore/config.yaml"]
