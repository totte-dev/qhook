# Deploy qhook to Fly.io

## 1. Initial Setup

```bash
fly launch --no-deploy          # Create app (don't deploy yet)
# Edit the generated fly.toml using the example below
fly deploy                      # Deploy
```

## 2. fly.toml Configuration

```toml
app = "your-qhook-app"
primary_region = "nrt"

[build]

[http_service]
  internal_port = 8888
  force_https = true
  auto_stop_machines = "off"    # Required: queue worker must stay running
  auto_start_machines = true
  min_machines_running = 1      # Required: keep at least 1 machine running

[mounts]
  source = "qhook_data"
  destination = "/data"

[env]
  QHOOK_CONFIG = "/data/qhook.yaml"
```

> **Important:** qhook runs a persistent queue worker process. You must set `auto_stop_machines = "off"` and `min_machines_running = 1`. Without these, the machine will stop and retry delivery will not execute.

## 3. Fly Postgres

```bash
fly postgres create --name qhook-db --region nrt
fly postgres attach qhook-db    # DATABASE_URL is automatically set as a Secret
```

Configure `qhook.yaml` to reference the `DATABASE_URL` environment variable.

## 4. Volumes (for SQLite)

SQLite requires a Volume for data persistence:

```bash
fly volumes create qhook_data --region nrt --size 1
```

Mount it via the `[mounts]` section in `fly.toml` (see above). Not needed when using Postgres (unless you store other data in `/data`).

## 5. Secrets

```bash
fly secrets set DATABASE_URL="postgres://..."   # If using Postgres (not needed after attach)
fly secrets set WEBHOOK_SECRET="your-secret"    # As needed
```

Check current secrets:

```bash
fly secrets list
```

## 6. Custom Domain

```bash
fly certs add your-domain.example.com
```

Configure the displayed CNAME / A records in your DNS. Certificates are issued automatically.

```bash
fly certs show your-domain.example.com   # Check certificate status
```
