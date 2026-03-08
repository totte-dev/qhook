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
