# Filter & Transform Example

Demonstrates event filtering and payload transformation -- two powerful features that reduce boilerplate in your handlers.

## Architecture

```
Events  --->  qhook  --[filter]--->  paid-only handler  (only status=paid)
                     --[transform]->  slack-notify       (reshaped payload)
                     --[both]------>  audit-log          (filtered + transformed)
```

## Getting Started

```bash
docker compose up
```

## Testing

**1. Send a paid order (passes all filters):**

```bash
curl -X POST http://localhost:8888/events/order.completed \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_001", "status": "paid", "customer": "alice", "amount": 4999, "currency": "jpy"}'
```

Expected: all 3 handlers receive the event.

**2. Send a pending order (filtered out by paid-only and audit-log):**

```bash
curl -X POST http://localhost:8888/events/order.completed \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_002", "status": "pending", "customer": "bob", "amount": 1200, "currency": "jpy"}'
```

Expected: only `slack-notify` receives the event (no filter).

**3. Send a refunded order:**

```bash
curl -X POST http://localhost:8888/events/order.completed \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_003", "status": "refunded", "customer": "carol", "amount": 3000, "currency": "jpy"}'
```

Expected: only `slack-notify` receives the event.

## Expected Output

```
[paid] order ord_001 from alice: 4999 jpy
[slack] {"text":"Order ord_001: alice paid 4999 jpy"}
[slack] {"text":"Order ord_002: bob pending 1200 jpy"}
[slack] {"text":"Order ord_003: carol refunded 3000 jpy"}
[audit] {"order_id":"ord_001","status":"paid","amount":4999}
```

## What This Shows

- **`filter`** -- Only process events matching a condition (e.g., `$.status == paid`)
- **`transform`** -- Reshape payloads before delivery (e.g., Slack message format)
- **Combined** -- Filter first, then transform the matching events
- Handlers without filter/transform receive the original payload unchanged
