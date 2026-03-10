---
layout: default
title: Deployment
---

# Deployment

qhook runs as a long-running process. Choose a platform that supports persistent services (not serverless/Lambda).

## Platform Comparison

| | AWS ECS | Fly.io | Railway | Render |
|---|---|---|---|---|
| **Best for** | Production at scale | Simple deployment | Quick prototyping | Easy setup |
| **Database** | RDS Postgres | Fly Postgres | Railway Postgres | Render Postgres |
| **SQLite support** | EFS volume | Fly Volume | Limited | Disk (Standard+) |
| **TLS** | ALB (auto) | Built-in | Built-in | Built-in |
| **Multi-instance** | Yes (Fargate) | Yes | No | No |
| **Cost** | Pay-per-use | From $0 | From $0 | From $0 |
| **Custom domain** | Route 53 / ALB | `fly certs` | Dashboard | Dashboard |

## Guides

- [AWS (ECS Fargate / EC2)](aws.md) -- production-grade with ALB, RDS, and optional nginx
- [Fly.io](flyio.md) -- simple deployment with Fly Postgres or SQLite volumes
- [Railway](railway.md) -- quick prototyping with auto-detected Dockerfile
- [Render](render.md) -- easy dashboard setup with auto-deploy from GitHub

## General Requirements

- **Port**: qhook listens on `8888` by default (configurable via `server.port` or `${PORT}`)
- **Health check**: `GET /health` returns `200` when healthy, `503` if DB is unreachable
- **Config file**: place `qhook.yaml` at `/data/qhook.yaml` (or set `QHOOK_CONFIG`)
- **Persistent process**: qhook must stay running for the queue worker to process deliveries -- do not use auto-stop/scale-to-zero

## Docker

```bash
# Development (SQLite)
docker compose up

# Production (Postgres)
DATABASE_URL=postgres://user:pass@db:5432/qhook docker compose -f docker-compose.prod.yaml up
```

The Docker image (`ghcr.io/totte-dev/qhook`) exposes port `8888` and expects a config file at `/data/qhook.yaml`.

## TLS / HTTPS

qhook does not terminate TLS itself. All deployment platforms above provide TLS termination:

- **AWS**: ALB handles HTTPS
- **Fly.io**: automatic TLS via `force_https = true`
- **Railway**: automatic HTTPS on all domains
- **Render**: automatic SSL via Let's Encrypt

For self-hosted (EC2, VPS), use a reverse proxy like Caddy (auto HTTPS) or nginx + Let's Encrypt. See the [AWS guide](aws.md#23-nginx-reverse-proxy) for an nginx example.
