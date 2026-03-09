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
| Custom | `hmac` | HMAC-SHA256 | `X-Webhook-Signature` |
| AWS SNS | (automatic) | X.509 RSA | (in body) |

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
