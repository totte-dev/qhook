---
layout: default
title: Examples
---

# Examples

Ready-to-run example projects demonstrating qhook features.

## quickstart

**The simplest possible setup.** No Docker, no external dependencies. Just the qhook binary, a Python receiver, and curl.

- [Source code](https://github.com/totte-dev/qhook/tree/main/examples/quickstart)
- Shows: event ingestion, automatic queuing, HTTP delivery

```bash
# Terminal 1: start qhook
qhook start -c examples/quickstart/qhook.yaml

# Terminal 2: start receiver
python3 examples/quickstart/receiver.py

# Terminal 3: send event
curl -X POST http://localhost:8888/events/order.created \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_001", "customer": "alice", "amount": 4999}'
```

---

## github-webhook

**GitHub push and pull request handling** with signature verification, event filtering, and payload transformation.

- [Source code](https://github.com/totte-dev/qhook/tree/main/examples/github-webhook)
- Shows: webhook verification, event routing, `filter`, `transform`

```bash
docker compose -f examples/github-webhook/docker-compose.yaml up
```

Push to `main` triggers deployment. PR events are transformed into Slack-formatted notifications. Locally testable via the `/events/` API.

---

## filter-transform

**Event filtering and payload transformation** with three handlers demonstrating different combinations.

- [Source code](https://github.com/totte-dev/qhook/tree/main/examples/filter-transform)
- Shows: `filter` (only paid orders), `transform` (Slack format), combined filter+transform

```bash
docker compose -f examples/filter-transform/docker-compose.yaml up
```

Send orders with different statuses and see how each handler responds differently.

---

## stripe-checkout

**Stripe checkout webhook processing** with fan-out to payment and fulfillment handlers.

- [Source code](https://github.com/totte-dev/qhook/tree/main/examples/stripe-checkout)
- Shows: Stripe signature verification, fan-out routing, idempotency keys, per-handler retry

```bash
docker compose -f examples/stripe-checkout/docker-compose.yaml up
```

One `checkout.session.completed` event is delivered to both the payment handler and the fulfillment handler, each with independent retry settings.

---

## workflow

**Multi-step order processing pipeline** with sequential execution, response chaining, and error routing.

- [Source code](https://github.com/totte-dev/qhook/tree/main/examples/workflow)
- Shows: workflows, step chaining, retry with error matching, catch routing

```bash
# Terminal 1: start qhook
qhook start -c examples/workflow/qhook.yaml

# Terminal 2: send event
curl -X POST http://localhost:8888/events/order.created \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_001", "amount": 5000}'
```

Three steps execute in sequence: validate → fulfill → notify. If validation returns 4xx, the catch block routes to handle-error instead.

---

## tenant-provision

**Multi-step tenant provisioning** with input validation, authenticated API calls, and reverse-order rollback.

- [Source code](https://github.com/totte-dev/qhook/tree/main/examples/tenant-provision)
- Shows: `params` (input validation), `headers` (auth to external APIs), reverse rollback chain

```bash
# Terminal 1: start qhook
qhook start -c examples/tenant-provision/qhook.yaml

# Terminal 2: send event
curl -X POST http://localhost:8888/events/tenant.create \
  -H "Authorization: Bearer test-token" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id": "t-123", "region": "us-east-1", "plan": "pro"}'
```

Provisions infrastructure (with auth header) → creates DB → configures services → activates. Missing required params are rejected. On failure, rollback runs in reverse order.

---

## alert-remediation

**PagerDuty alert auto-remediation** with severity-based routing, wait-and-verify, and API escalation.

- [Source code](https://github.com/totte-dev/qhook/tree/main/examples/alert-remediation)
- Shows: `verify: pagerduty`, `choice` step, `wait` step, `headers` for PagerDuty API

```bash
# Terminal 1: start qhook
PAGERDUTY_WEBHOOK_SECRET=my-secret qhook start -c examples/alert-remediation/qhook.yaml

# Terminal 2: send simulated alert
SECRET=my-secret
PAYLOAD='{"event":{"event_type":"incident.triggered"},"severity":"critical"}'
SIG=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$SECRET" | cut -d' ' -f2)
curl -X POST http://localhost:8888/webhooks/pagerduty \
  -H "Content-Type: application/json" \
  -H "X-PagerDuty-Signature: v1=$SIG" \
  -d "$PAYLOAD"
```

Routes by severity: critical → restart, warning → scale-up, default → notify. After remediation, waits 30s and runs a health check. If it fails, escalates via the PagerDuty API with authentication.
