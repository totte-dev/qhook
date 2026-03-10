---
layout: default
title: Local Development
---

# Local Development

This guide covers the fastest path from setup to a working local development environment.

## Quick start

```bash
# Install and initialize
cargo install qhook
qhook init

# Start the server (uses SQLite by default)
qhook start
```

That's it. qhook is now receiving events on `http://localhost:8080`.

## Built-in echo endpoint

Every qhook instance exposes `/_echo` — a built-in endpoint that returns the request body and headers as JSON. Use it as a handler URL during development instead of running a separate mock server.

```yaml
sources:
  app:
    type: event

handlers:
  debug:
    source: app
    events: ["*"]
    url: http://localhost:8080/_echo
```

Send a test event and the echo endpoint returns exactly what was delivered:

```bash
qhook send -s app -t order.created '{"id": "123"}'
```

The echo response:

```json
{
  "headers": { "content-type": "application/json", ... },
  "body": { "id": "123" }
}
```

## Sending test events

Use `qhook send` to send events without manually constructing curl commands:

```bash
# Inline JSON payload
qhook send -s app -t order.created '{"id": "123", "amount": 500}'

# From a file
qhook send -s app -t user.signup -f test-payload.json

# Empty payload (default: {})
qhook send -s app -t health.ping
```

The command reads your config to determine the correct endpoint path, port, and authentication.

## Pre-production checks

Run `qhook doctor` before deploying to verify your setup:

```bash
$ qhook doctor
  Config valid (qhook.yaml)
  Database connection OK (postgres)
  Server reachable at localhost:8080
  3 handler endpoint(s) reachable
  All checks passed.
```

Doctor checks:
- Config file syntax and validation
- Database connectivity
- Server health endpoint
- Handler and workflow step endpoint reachability
- Security settings (SSRF protection, auth tokens)

## Receiving external webhooks locally

### GitHub webhooks

Use the GitHub CLI's built-in webhook forwarding:

```bash
gh webhook forward \
  --repo=owner/repo \
  --events=push,pull_request \
  --url=http://localhost:8080/webhooks/github
```

Signature verification works as-is — GitHub signs the forwarded payloads.

### AWS SNS

Use [LocalStack](https://localstack.cloud/) for local SNS testing:

```bash
docker run -d --name localstack -p 4566:4566 localstack/localstack

# Create topic and subscribe
aws --endpoint-url=http://localhost:4566 sns create-topic --name my-topic
aws --endpoint-url=http://localhost:4566 sns subscribe \
  --topic-arn arn:aws:sns:us-east-1:000000000000:my-topic \
  --protocol http \
  --notification-url http://host.docker.internal:8080/sns/my-source
```

Set `skip_verify: true` on the source to skip X.509 signature verification with LocalStack:

```yaml
sources:
  my-source:
    type: sns
    skip_verify: true
```

### Stripe, PagerDuty, and other providers

Use [cloudflared](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/) for a free tunnel with no account required:

```bash
brew install cloudflared   # or apt, scoop, etc.
cloudflared tunnel --url http://localhost:8080
# => https://random-name.trycloudflare.com
```

Use the generated URL as your webhook endpoint in the provider's dashboard. Signature verification works normally since the original headers are preserved.

## Development workflow

A typical dev cycle:

```bash
# 1. Start qhook with echo handler
qhook start

# 2. Send test events
qhook send -s app -t order.created '{"id": "1"}'

# 3. Check results
qhook jobs list
qhook events list

# 4. When ready for production
qhook doctor -c production.yaml
```
