FROM rust:1.94-slim

RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev git curl python3 \
    awscli jq \
    && rm -rf /var/lib/apt/lists/*

# Rust tools
RUN rustup component add clippy rustfmt

WORKDIR /workspace
