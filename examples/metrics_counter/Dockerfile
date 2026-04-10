FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY cfgsync/ ./cfgsync/
COPY examples/ ./examples/
COPY testing-framework/ ./testing-framework/

RUN cargo build --release -p metrics-counter-node

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/metrics-counter-node /usr/local/bin/metrics-counter-node

RUN mkdir -p /etc/metrics-counter
WORKDIR /app

ENTRYPOINT ["/usr/local/bin/metrics-counter-node"]
CMD ["--config", "/etc/metrics-counter/config.yaml"]
