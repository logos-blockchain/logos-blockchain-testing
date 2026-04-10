FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY cfgsync/ ./cfgsync/
COPY examples/ ./examples/
COPY testing-framework/ ./testing-framework/

RUN cargo build --release -p scheduler-node

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/scheduler-node /usr/local/bin/scheduler-node

RUN mkdir -p /etc/scheduler
WORKDIR /app

ENTRYPOINT ["/usr/local/bin/scheduler-node"]
CMD ["--config", "/etc/scheduler/config.yaml"]
