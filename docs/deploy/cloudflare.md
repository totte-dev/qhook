---
layout: default
title: Deploy to Cloudflare Containers
---

# Deploy qhook to Cloudflare Containers

Cloudflare Containers run Docker images on Cloudflare's network with low-latency access to D1 (SQLite-compatible database). qhook supports D1 natively via its `d1` database driver.

## Prerequisites

- Cloudflare account with Containers access
- [Wrangler CLI](https://developers.cloudflare.com/workers/wrangler/) v3+
- A D1 database (`wrangler d1 create qhook`)
- Docker (for building the image)

---

## 1. Create a D1 Database

```bash
wrangler d1 create qhook
```

Note the `database_id` from the output -- you will need it for the config.

## 2. D1 Connection Modes

qhook connects to D1 in two ways:

| Mode | Config | Best for |
|------|--------|----------|
| **REST API** | `account_id` + `api_token` + `database_id` | Running outside Cloudflare (local dev, other clouds) |
| **Outbound Workers proxy** | `d1_endpoint` + `database_id` | Running inside Cloudflare Containers (lower latency, no API token needed) |

### REST API Mode

qhook calls the Cloudflare D1 REST API directly. Works from anywhere but requires an API token with D1 read/write permissions.

```yaml
database:
  driver: d1
  database_id: ${CF_D1_DB_ID}
  account_id: ${CF_ACCOUNT_ID}
  api_token: ${CF_API_TOKEN}
```

### Outbound Workers Proxy Mode

When running inside Cloudflare Containers, you can bind D1 to an Outbound Worker that proxies SQL queries. This avoids REST API overhead and does not require an API token.

```yaml
database:
  driver: d1
  database_id: ${CF_D1_DB_ID}
  d1_endpoint: http://d1.local
```

The `d1_endpoint` points to the Outbound Worker's internal address. See the wrangler.toml example below for the binding configuration.

## 3. qhook.yaml

Full config example for Cloudflare Containers with D1:

```yaml
server:
  port: 8888

database:
  driver: d1
  database_id: ${CF_D1_DB_ID}
  # REST API mode (uncomment if not using Outbound Workers proxy):
  # account_id: ${CF_ACCOUNT_ID}
  # api_token: ${CF_API_TOKEN}
  # Proxy mode (recommended for Containers):
  d1_endpoint: http://d1.local

sources:
  github:
    type: webhook
    path: /webhook/github
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}

handlers:
  notify:
    source: github
    events: [push]
    url: https://your-service.example.com/notify
    retry: { max: 5 }
```

## 4. Dockerfile

The existing qhook Dockerfile works as-is for Cloudflare Containers:

```dockerfile
FROM ghcr.io/totte-dev/qhook:latest
COPY qhook.yaml /data/qhook.yaml
```

Or build from source:

```dockerfile
FROM rust:1.94-slim AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release 2>/dev/null || true && rm -rf src
COPY src/ src/
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/qhook /usr/local/bin/qhook
RUN mkdir -p /data
WORKDIR /data
COPY qhook.yaml /data/qhook.yaml
ENV RUST_LOG=qhook=info
EXPOSE 8888
ENTRYPOINT ["qhook"]
CMD ["start", "--config", "/data/qhook.yaml"]
```

## 5. wrangler.toml

```toml
name = "qhook"
compatibility_date = "2024-12-01"

[containers]
image = "./Dockerfile"
max_instances = 1

[containers.port]
http = 8888

[[d1_databases]]
binding = "QHOOK_DB"
database_name = "qhook"
database_id = "your-d1-database-id"

# Outbound Worker proxy for D1 (recommended)
# The container accesses D1 via http://d1.local
# See: https://developers.cloudflare.com/containers/
```

> **Note:** Cloudflare Containers configuration may evolve. Refer to the [Cloudflare Containers documentation](https://developers.cloudflare.com/containers/) for the latest wrangler.toml schema and binding syntax.

## 6. Deploy

```bash
# Build and deploy
wrangler deploy
```

## 7. Secrets

Store sensitive values as secrets rather than plaintext in wrangler.toml:

```bash
wrangler secret put GITHUB_WEBHOOK_SECRET
wrangler secret put CF_API_TOKEN          # Only needed for REST API mode
```

Secrets are available as environment variables inside the container. Reference them in `qhook.yaml` with `${VAR}` syntax.

## 8. Custom Domain

Configure a custom domain in the Cloudflare dashboard under **Workers & Pages > your container > Settings > Domains & Routes**.

Cloudflare provides automatic TLS termination -- no additional configuration needed.

## 9. Health Check

qhook exposes `GET /health` which returns `200` when healthy. Configure this as the health check endpoint in your container settings.

---

## Limitations

- **D1 is SQLite-based**: D1 supports a single writer at a time. For high-throughput production deployments, consider Postgres or MySQL on a dedicated database host.
- **Cloudflare Containers are in beta**: Check the [Cloudflare Containers documentation](https://developers.cloudflare.com/containers/) for current availability and limitations.
- **Persistent process required**: qhook must stay running for the queue worker to process deliveries. Set `max_instances = 1` (minimum) and ensure the container is not auto-stopped.
