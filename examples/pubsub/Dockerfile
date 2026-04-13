FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY cfgsync/ ./cfgsync/
COPY examples/ ./examples/
COPY testing-framework/ ./testing-framework/

RUN cargo build --release -p pubsub-node

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/pubsub-node /usr/local/bin/pubsub-node

RUN mkdir -p /etc/pubsub
WORKDIR /app

ENTRYPOINT ["/usr/local/bin/pubsub-node"]
CMD ["--config", "/etc/pubsub/config.yaml"]
