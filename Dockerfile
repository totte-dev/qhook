# Stage 1: Build
FROM rust:1.98-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
# Cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release 2>/dev/null || true && rm -rf src

COPY src/ src/
RUN touch src/main.rs && cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/qhook /usr/local/bin/qhook

RUN mkdir -p /data
WORKDIR /data

ENV RUST_LOG=qhook=info

EXPOSE 8888

ENTRYPOINT ["qhook"]
CMD ["start", "--config", "/data/qhook.yaml"]
