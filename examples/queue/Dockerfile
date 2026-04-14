FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY cfgsync/ ./cfgsync/
COPY examples/ ./examples/
COPY testing-framework/ ./testing-framework/

RUN cargo build --release -p queue-node

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/queue-node /usr/local/bin/queue-node

RUN mkdir -p /etc/queue
WORKDIR /app

ENTRYPOINT ["/usr/local/bin/queue-node"]
CMD ["--config", "/etc/queue/config.yaml"]
