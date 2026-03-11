# Outbound Webhooks

Send webhooks to your customers with HMAC-SHA256 signatures. Like Svix, but built into qhook.

## Setup

1. Start qhook:

```bash
qhook start -c examples/outbound-webhook/qhook.yaml
```

2. Register a customer endpoint:

```bash
curl -s -X POST http://localhost:8888/api/outbound/endpoints \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"source": "my-saas", "url": "http://localhost:9000/webhook", "description": "Customer A"}'
```

Save the `signing_secret` from the response (starts with `whsec_`).

3. Subscribe the endpoint to events:

```bash
curl -s -X POST http://localhost:8888/api/outbound/endpoints/{ENDPOINT_ID}/subscriptions \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"event_types": ["order.created", "payment.completed"]}'
```

4. Start the customer receiver (pass the signing secret):

```bash
python3 examples/outbound-webhook/receiver.py whsec_...
```

5. Send an event from your app:

```bash
curl -X POST http://localhost:8888/events/my-saas/order.created \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"order_id": "ord_001", "customer": "alice", "amount": 4999}'
```

## Expected Output

The customer receiver logs:

```
[VERIFIED] event_type=order.created event_id=01JXXXX delivery_id=01JYYYY payload={"order_id": "ord_001", "customer": "alice", "amount": 4999}
```

## What This Shows

- **Dynamic endpoint registration** via Management API
- **Per-endpoint signing secrets** with `whsec_` prefix
- **HMAC-SHA256 signed delivery** with `X-Qhook-Signature: v1=...`
- **Subscription-based routing** -- only subscribed event types are delivered
- **Automatic retry** with exponential backoff on failure

## Management API

| Operation | Command |
|-----------|---------|
| List endpoints | `curl -H "Authorization: Bearer $TOKEN" http://localhost:8888/api/outbound/endpoints` |
| Disable endpoint | `curl -X PUT -H "Authorization: Bearer $TOKEN" -d '{"status":"disabled"}' http://localhost:8888/api/outbound/endpoints/{id}` |
| Rotate secret | `curl -X POST -H "Authorization: Bearer $TOKEN" http://localhost:8888/api/outbound/endpoints/{id}/rotate-secret` |
| Delete endpoint | `curl -X DELETE -H "Authorization: Bearer $TOKEN" http://localhost:8888/api/outbound/endpoints/{id}` |
