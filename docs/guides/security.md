---
layout: default
title: Security
---

# Security

## Authentication

### Bearer Token

Protect the `/events` endpoint with a bearer token:

```yaml
api:
  auth_token: ${QHOOK_API_TOKEN}
```

```bash
curl -X POST http://localhost:8888/events/order.created \
  -H "Authorization: Bearer your-token-here" \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_123"}'
```

Requests without a valid token receive `401 Unauthorized`. Token comparison uses **constant-time equality** to prevent timing attacks.

If `auth_token` is not configured, qhook logs a warning at startup reminding you to secure the endpoint in production.

### Metrics Endpoint

Protect the `/metrics` endpoint with a bearer token:

```yaml
api:
  metrics_auth_token: ${QHOOK_METRICS_TOKEN}
```

Without this, the `/metrics` endpoint is open and exposes operational details (queue depth, handler names, error rates). Configure this in production or restrict access at the network level.

### Webhook Signatures

External webhook sources are protected via provider-specific signature verification. See [Webhook Verification](webhook-verification.md).

## Request Limits

### Body Size Limit

```yaml
server:
  max_body_size: 1048576    # 1MB (default)
```

Prevents memory exhaustion from oversized payloads. Requests exceeding the limit receive `413 Payload Too Large`.

### Concurrency Limit

```yaml
server:
  max_inbound: 100          # default
```

Returns `503 Service Unavailable` when exceeded. Protects qhook and its database from overload.

### Per-IP Rate Limiting

```yaml
server:
  ip_rate_limit: 100        # requests/sec per IP (default: 0 = disabled)
  trust_proxy: true          # use X-Forwarded-For when behind a reverse proxy
```

Returns `429 Too Many Requests` when exceeded. Sliding window counter per IP address. Capped at 100K tracked IPs to prevent memory exhaustion; new IPs beyond the cap are rate-limited by default.

**Important:** When running behind a reverse proxy (nginx, ALB, etc.), set `trust_proxy: true` so that rate limiting uses the real client IP from the `X-Forwarded-For` or `X-Real-IP` header instead of the proxy's IP. Without this, all requests appear to come from the proxy and rate limiting is ineffective.

### SSRF Protection

Handler and workflow step URLs are validated against private/loopback IP ranges by default:

```yaml
server:
  allow_private_urls: true   # allow localhost/private IPs (dev only)
```

Without `allow_private_urls`, URLs pointing to `localhost`, `127.0.0.1`, `10.x.x.x`, `172.16-31.x.x`, `192.168.x.x`, and `169.254.x.x` are rejected at config validation time. This prevents SSRF attacks (e.g., accessing cloud metadata at `169.254.169.254`).

Enable `allow_private_urls` only for local development or when handlers run on the same host.

## Security Headers

All responses include:

| Header | Value | Purpose |
|--------|-------|---------|
| `X-Content-Type-Options` | `nosniff` | Prevent MIME type sniffing |
| `X-Frame-Options` | `DENY` | Prevent clickjacking |
| `Cache-Control` | `no-store` | Prevent caching of webhook data |

## TLS

qhook does not terminate TLS itself. Use a reverse proxy in front of qhook for HTTPS:

- **Caddy** -- automatic HTTPS with Let's Encrypt
- **nginx** -- see [AWS deployment guide](../deploy/aws.md) for an nginx config example
- **Cloud load balancer** -- ALB (AWS), Cloud Load Balancing (GCP), etc.

For self-hosted deployments, see the nginx reverse proxy example in the [AWS deployment guide](../deploy/aws.md#23-nginx-reverse-proxy).

## Handler URL Validation

At config load, qhook validates handler URLs:

- **Scheme check**: only `http://` and `https://` are accepted
- **Private IP warning**: URLs pointing to private IP ranges (`10.x`, `172.16-31.x`, `192.168.x`) trigger a warning, as they may indicate misconfiguration or SSRF risk

## SNS Security

- **SubscribeURL validation**: before fetching a `SubscribeURL`, qhook validates it points to a legitimate SNS domain, preventing SSRF
- **Certificate URL validation**: only certificates hosted on `sns.*.amazonaws.com` are accepted
- **Certificate caching**: certificates are cached for 1 hour to prevent DoS via repeated fetches

## Best Practices

1. **Always set `auth_token`** for the `/events` endpoint in production
2. **Use environment variables** for secrets (`${VAR}` syntax) -- never commit secrets to config files
3. **Enable `ip_rate_limit`** to protect against abuse
4. **Use HTTPS** via a reverse proxy or cloud load balancer
5. **Set `max_body_size`** appropriate to your payload sizes
6. **Monitor `qhook_verification_failures_total`** for signature verification anomalies
7. **Set up alerts** on DLQ and verification failures
