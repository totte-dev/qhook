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

### Custom HMAC

```yaml
sources:
  my-service:
    type: webhook
    verify: hmac
    secret: ${MY_WEBHOOK_SECRET}
```

Checks the `X-Webhook-Signature` header using HMAC-SHA256 (hex-encoded). Use this for any service that sends an HMAC signature in a custom header.

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
