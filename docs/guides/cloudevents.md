---
layout: default
title: CloudEvents
---

# CloudEvents

qhook automatically detects and handles [CloudEvents](https://cloudevents.io/) in both content modes. No configuration needed.

## Binary Mode

CloudEvents metadata is sent as HTTP headers. The body contains the event data directly.

```bash
curl -X POST http://localhost:8888/events/ignored \
  -H "Content-Type: application/json" \
  -H "ce-type: com.example.order.created" \
  -H "ce-source: /shop" \
  -H "ce-id: evt-001" \
  -H "ce-specversion: 1.0" \
  -d '{"orderId": "ord_123"}'
```

The `ce-type` header overrides the event type from the URL path. In this example, the event type is `com.example.order.created` (not `ignored`).

## Structured Mode

The entire CloudEvents envelope is sent as JSON with `Content-Type: application/cloudevents+json`.

```bash
curl -X POST http://localhost:8888/webhooks/my-source \
  -H "Content-Type: application/cloudevents+json" \
  -d '{
    "specversion": "1.0",
    "type": "com.example.order.created",
    "source": "/shop",
    "id": "evt-001",
    "data": {"orderId": "ord_123"}
  }'
```

The `type` field from the envelope is used as the event type.

## Header Forwarding

All `ce-*` headers from the original event are stored with the event and **automatically forwarded** to your handlers on delivery. Your application can access CloudEvents metadata without parsing the payload.

```python
@app.route("/jobs/order", methods=["POST"])
def handle_order():
    ce_type = request.headers.get("ce-type")       # com.example.order.created
    ce_source = request.headers.get("ce-source")    # /shop
    ce_id = request.headers.get("ce-id")            # evt-001
    payload = request.get_json()                     # {"orderId": "ord_123"}
    # ...
```

## Detection Priority

qhook extracts the event type in this order:

1. **`ce-type` header** (binary mode CloudEvents)
2. **`type` field** in the JSON body (structured mode CloudEvents)
3. **URL path** (e.g., `/events/order.created` -> `order.created`)
4. **Provider-specific logic** (e.g., SNS Subject field)
