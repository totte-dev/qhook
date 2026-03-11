---
layout: default
title: Webhook Verification
---

# Webhook Verification

qhook verifies inbound webhook signatures before processing. This ensures payloads are authentic and haven't been tampered with.

## Supported Providers

### GitHub

```yaml
sources:
  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}
```

Checks the `X-Hub-Signature-256` header using HMAC-SHA256.

**GitHub setup:** In your repository settings, go to Webhooks > Add webhook. Set the Payload URL to `https://your-host/webhooks/github`, Content type to `application/json`, and enter the same secret.

### Stripe

```yaml
sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}
```

Checks the `Stripe-Signature` header (`t=...,v1=...` format) using HMAC-SHA256 with timestamp. **Replay protection:** signatures older than 5 minutes are rejected.

**Stripe setup:** In the Stripe Dashboard, go to Developers > Webhooks > Add endpoint. Set the URL to `https://your-host/webhooks/stripe`. Copy the signing secret (`whsec_...`) to your config.

### Shopify

```yaml
sources:
  shopify:
    type: webhook
    verify: shopify
    secret: ${SHOPIFY_WEBHOOK_SECRET}
```

Checks the `X-Shopify-Hmac-SHA256` header using HMAC-SHA256 (base64-encoded).

### PagerDuty

```yaml
sources:
  pagerduty:
    type: webhook
    verify: pagerduty
    secret: ${PAGERDUTY_WEBHOOK_SECRET}
```

Checks the `X-PagerDuty-Signature` header (`v1=...` format) using HMAC-SHA256.

**PagerDuty setup:** In PagerDuty, go to Integrations > Generic Webhooks (v3). Add a subscription with the URL `https://your-host/webhooks/pagerduty`. Copy the signing secret to your config.

### Grafana

```yaml
sources:
  grafana:
    type: webhook
    verify: grafana
    secret: ${GRAFANA_WEBHOOK_SECRET}
```

Checks the `X-Grafana-Alerting-Signature` header using HMAC-SHA256.

**Grafana setup:** In Grafana, go to Alerting > Contact points. Add a webhook contact point with the URL `https://your-host/webhooks/grafana`. Under Optional Webhook Settings, enter the same secret.

### Terraform Cloud

```yaml
sources:
  terraform:
    type: webhook
    verify: terraform
    secret: ${TF_NOTIFICATION_SECRET}
```

Checks the `X-TFE-Notification-Signature` header using HMAC-SHA512.

**Terraform Cloud setup:** In your workspace settings, go to Notifications > Create a Notification. Set the destination URL to `https://your-host/webhooks/terraform` and enter a token (used as the HMAC secret).

### GitLab

```yaml
sources:
  gitlab:
    type: webhook
    verify: gitlab
    secret: ${GITLAB_WEBHOOK_TOKEN}
```

Compares the `X-Gitlab-Token` header directly against the configured secret (constant-time comparison).

**GitLab setup:** In your project settings, go to Webhooks > Add new webhook. Set the URL to `https://your-host/webhooks/gitlab` and enter the secret token.

### Custom HMAC

```yaml
sources:
  my-service:
    type: webhook
    verify: hmac
    secret: ${MY_WEBHOOK_SECRET}
```

Checks the `X-Webhook-Signature` header using HMAC-SHA256 (hex-encoded). Use this for any service that sends an HMAC signature in a custom header.

### Standard Webhooks (Clerk, Resend, Lemon Squeezy, etc.)

```yaml
sources:
  clerk:
    type: webhook
    verify: standard-webhooks
    secret: ${CLERK_WEBHOOK_SECRET}    # whsec_... from provider dashboard
```

Checks `webhook-id`, `webhook-timestamp`, and `webhook-signature` headers per the [Standard Webhooks](https://www.standardwebhooks.com/) specification. HMAC-SHA256 with base64 encoding. Includes **replay protection** (5-minute timestamp tolerance). Supports multiple signatures (space-separated) for key rotation.

Use this for any provider that follows the Standard Webhooks spec, including Clerk, Resend, Lemon Squeezy, and Orb.

### Linear

```yaml
sources:
  linear:
    type: webhook
    verify: linear
    secret: ${LINEAR_WEBHOOK_SECRET}
```

Checks the `Linear-Signature` header using HMAC-SHA256 (hex-encoded).

**Linear setup:** In Linear, go to Settings > API > Webhooks > New Webhook. Set the URL to `https://your-host/webhooks/linear` and copy the signing secret.

## Provider Summary

| Provider | `verify` value | Algorithm | Header |
|----------|---------------|-----------|--------|
| GitHub | `github` | HMAC-SHA256 | `X-Hub-Signature-256` |
| Stripe | `stripe` | HMAC-SHA256 + timestamp | `Stripe-Signature` |
| Shopify | `shopify` | HMAC-SHA256 (base64) | `X-Shopify-Hmac-SHA256` |
| PagerDuty | `pagerduty` | HMAC-SHA256 | `X-PagerDuty-Signature` |
| Grafana | `grafana` | HMAC-SHA256 | `X-Grafana-Alerting-Signature` |
| Terraform Cloud | `terraform` | HMAC-SHA512 | `X-TFE-Notification-Signature` |
| GitLab | `gitlab` | Token comparison | `X-Gitlab-Token` |
| Standard Webhooks | `standard-webhooks` | HMAC-SHA256 (base64) + timestamp | `webhook-signature` |
| Linear | `linear` | HMAC-SHA256 | `Linear-Signature` |
| Custom | `hmac` | HMAC-SHA256 | `X-Webhook-Signature` |
| AWS SNS | (automatic) | X.509 RSA | (in body) |

## Outbound Webhook Signing (Standard Webhooks)

When using `type: outbound` sources, qhook signs outgoing deliveries using the [Standard Webhooks](https://www.standardwebhooks.com/) specification. Each delivery includes three headers:

| Header | Description |
|--------|-------------|
| `webhook-id` | Unique message identifier (ULID) |
| `webhook-timestamp` | Unix timestamp (seconds since epoch) |
| `webhook-signature` | `v1,{base64-encoded HMAC-SHA256}` |

**Signed content format:** `{msg_id}.{timestamp}.{body}`

Each endpoint gets a unique signing secret (starts with `whsec_`). The `whsec_` prefix is stripped and the remaining value is base64-decoded to get the raw HMAC key bytes.

### Verifying in Your Application

```python
import hmac, hashlib, base64

def verify(secret: str, webhook_id: str, timestamp: str, body: bytes, signature: str) -> bool:
    # Strip whsec_ prefix and base64-decode to get raw key
    key = base64.b64decode(secret.removeprefix("whsec_"))
    # Build signed content: msg_id.timestamp.body
    signed_content = f"{webhook_id}.{timestamp}.".encode() + body
    expected = base64.b64encode(
        hmac.new(key, signed_content, hashlib.sha256).digest()
    ).decode()
    # Compare against v1,{signature} format
    return hmac.compare_digest(signature.removeprefix("v1,"), expected)
```

### Managing Outbound Endpoints

```bash
# Register a customer endpoint
curl -X POST http://localhost:8888/api/outbound/endpoints \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"source": "my-saas", "url": "https://customer.com/webhook"}'
# → {"id": "01J...", "signing_secret": "whsec_..."}

# Subscribe to event types
curl -X POST http://localhost:8888/api/outbound/endpoints/{id}/subscriptions \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"event_types": ["order.created", "payment.completed"]}'

# Rotate signing secret
curl -X POST http://localhost:8888/api/outbound/endpoints/{id}/rotate-secret \
  -H "Authorization: Bearer $TOKEN"
```

> See the [outbound webhook example](https://github.com/totte-dev/qhook/tree/main/examples/outbound-webhook) for a complete walkthrough.

## Security Notes

- All signature comparisons use **constant-time equality** (`subtle::ct_eq`) to prevent timing attacks.
- The `secret` field is required when `verify` is set. Config validation fails without it.
- Use environment variables (`${VAR}`) for secrets -- never commit them to your config file.

## Testing Without Verification

For local development, use `type: event` sources (no signature check) and send events via the `/events/` endpoint:

```bash
curl -X POST http://localhost:8888/events/order.created \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_123"}'
```

In production, always use `type: webhook` with `verify` for external providers.
