---
layout: default
title: Deploy to Railway
---

# Deploy qhook to Railway

> Railway's free plan has execution hour limits, but works well for testing.

## 1. Deploy via Railway CLI

```bash
# Install CLI
npm install -g @railway/cli

# Login & create project
railway login
railway init

# Deploy (auto-detects Dockerfile)
railway up
```

Railway detects the `Dockerfile` in the repository root and builds automatically.

## 2. Environment Variables

Set via the Railway dashboard or CLI:

```bash
railway variables set RUST_LOG=info
railway variables set DATABASE_URL=sqlite:///data/qhook.db
```

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | DB connection string | `postgres://...` or `sqlite:///data/qhook.db` |
| `RUST_LOG` | Log level | `info`, `debug` |
| `PORT` | Auto-set by Railway | (automatic) |

## 3. Postgres Add-on

```bash
railway add --plugin postgresql
```

This automatically injects `DATABASE_URL` into your environment variables. No additional qhook configuration needed. If switching from SQLite, remove the manually set `DATABASE_URL`.

## 4. Custom Domain

```bash
railway domain  # Creates a *.up.railway.app subdomain
```

For custom domains, add them in the dashboard under **Settings > Domains** and configure a DNS CNAME record.

## 5. Config File Placement

Railway has limited volume persistence, so use one of these approaches:

### Option A: Include in repository (recommended)

Place `qhook.yaml` in your repo and copy it in the `Dockerfile`:

```dockerfile
COPY qhook.yaml /data/qhook.yaml
```

### Option B: Railway Volumes

Mount a Volume to `/data` in the dashboard and manually place the file on first deploy.

### PORT Environment Variable

Railway may specify the listen port via the `PORT` environment variable. Handle this in `qhook.yaml`:

```yaml
server:
  port: ${PORT:-8888}
```

Falls back to `8888` if `PORT` is not set.

---

Minimal setup: `railway init && railway up`. Add the Postgres add-on when you need it.
