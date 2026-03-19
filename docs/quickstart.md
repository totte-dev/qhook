---
layout: default
title: 5-Minute Quickstart
---

# 5-Minute Quickstart

Get qhook receiving and processing Stripe webhooks in under 5 minutes. No Docker, no external database.

## 1. Install

```bash
cargo install qhook
```

Or download the binary from [GitHub Releases](https://github.com/totte-dev/qhook/releases).

## 2. Create config

```bash
cat > qhook.yaml << 'EOF'
database:
  driver: sqlite

server:
  port: 8888
  allow_private_urls: true

sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}

handlers:
  billing:
    source: stripe
    events: [invoice.paid, customer.subscription.updated]
    url: http://localhost:3000/webhook
    idempotency_key: "$.id"
    retry: { max: 5 }
EOF
```

## 3. Start qhook

```bash
export STRIPE_WEBHOOK_SECRET=whsec_test_xxx
qhook start
```

qhook is now listening on `http://localhost:8888`. Incoming Stripe webhooks are signature-verified, persisted to SQLite, and delivered to your handler at `localhost:3000/webhook`.

## 4. Send a test event

Open another terminal and send a test event via the internal API (bypasses signature verification):

```bash
curl -X POST http://localhost:8888/events/stripe/invoice.paid \
  -H "Content-Type: application/json" \
  -d '{"id": "evt_test_001", "type": "invoice.paid", "data": {"object": {"customer": "cus_abc", "amount_paid": 4900}}}'
```

You should see:

```json
{"event_id": "01J...", "jobs_created": 1}
```

## 5. Check status

```bash
# Health
curl http://localhost:8888/health

# Events received
qhook events list

# Jobs (delivery attempts)
qhook jobs list

# Stream events in real time
qhook tail
```

## What just happened?

```
curl (test event)
  ↓
qhook received → persisted to SQLite → ACK (202)
  ↓
queue worker → POST http://localhost:3000/webhook
  ↓ success? → mark completed
  ↓ failure? → retry with exponential backoff (up to 5 times)
  ↓ all retries failed? → Dead Letter Queue
```

## Next: connect real Stripe webhooks

Use Stripe CLI to forward test webhooks from Stripe to your local qhook:

```bash
stripe listen --forward-to localhost:8888/webhooks/stripe
```

Stripe CLI prints a webhook signing secret (`whsec_...`). Use that as your `STRIPE_WEBHOOK_SECRET`.

## Next: add a workflow

Turn a single event into a multi-step pipeline:

```yaml
workflows:
  onboard:
    source: stripe
    events: [customer.subscription.created]
    timeout: 120
    steps:
      - name: provision
        url: http://localhost:3000/provision
        retry: { max: 3 }
      - name: welcome-email
        url: http://localhost:3000/email
        catch:
          - errors: [all]
            goto: notify-failure
        end: true
      - name: notify-failure
        url: http://localhost:3000/alert
        end: true
```

## Next steps

- [Configuration Reference](configuration.md) — all config options
- [Webhook Verification](guides/webhook-verification.md) — 13 supported providers
- [Workflows](guides/workflows.md) — branching, parallelism, rollback
- [Deployment](deploy/) — deploy to AWS, Fly.io, Railway, Render

> **qhook Cloud** (managed) is coming soon. Same config, zero ops. [Sign up for early access](https://qhook.dev).
