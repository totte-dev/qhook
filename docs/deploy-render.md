# Deploy qhook to Render

## 1. Create a Web Service

1. In the Render dashboard, select **New > Web Service**
2. Connect your GitHub repository and select the qhook repo
3. Configure:
   - **Environment**: Docker (Dockerfile is auto-detected)
   - **Region**: Closest to your users
   - **Instance Type**: Starter or above

## 2. Environment Variables

Set in the **Environment** tab:

| Variable | Value | Notes |
|----------|-------|-------|
| `PORT` | `8888` | Render may auto-set this |
| `DATABASE_URL` | `postgres://...` or `sqlite:///data/qhook.db` | See below |
| `RUST_LOG` | `info` | Use `debug` for verbose logs |

### PORT Mapping

Render specifies the port via the `PORT` environment variable. Reference it in `qhook.yaml`:

```yaml
server:
  port: ${PORT}
```

Keep the Dockerfile `EXPOSE` consistent:

```dockerfile
EXPOSE 8888
```

## 3. Postgres Add-on

1. In the Render dashboard, create **New > PostgreSQL**
2. Copy the **Internal Database URL**
3. Set it as `DATABASE_URL` in the Web Service environment variables

Traffic goes through Render's internal network -- no external exposure needed.

## 4. Disk (for SQLite)

Render's filesystem resets on every deploy, so SQLite requires a **Disk**:

1. Go to the Web Service **Disks** tab and click **Add Disk**
2. Configure:
   - **Mount Path**: `/data`
   - **Size**: 1 GB (increase as needed)

`qhook.yaml` and the SQLite DB are persisted under `/data`.

> **Note**: Starter plan does not support Disks. Standard plan or above is required for SQLite.

## 5. Health Check

Set the **Health Check Path** in the **Settings** tab:

```
/health
```

Render periodically calls this endpoint and auto-restarts on failure.

## 6. Custom Domain

1. Go to **Settings > Custom Domains** and click **Add Custom Domain**
2. Enter your domain (e.g., `hooks.example.com`)
3. Add the displayed CNAME record to your DNS:
   ```
   hooks.example.com  CNAME  xxx.onrender.com
   ```
4. SSL certificates are auto-provisioned (Let's Encrypt)

---

Deploys trigger automatically on push to GitHub. For manual deploys, use **Manual Deploy > Deploy latest commit**.
