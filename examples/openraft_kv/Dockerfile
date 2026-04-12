# Build stage
FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY cfgsync/ ./cfgsync/
COPY examples/ ./examples/
COPY testing-framework/ ./testing-framework/

RUN cargo build --release -p openraft-kv-node

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/openraft-kv-node /usr/local/bin/openraft-kv-node

RUN mkdir -p /etc/openraft-kv
WORKDIR /app

ENTRYPOINT ["/usr/local/bin/openraft-kv-node"]
CMD ["--config", "/etc/openraft-kv/config.yaml"]
