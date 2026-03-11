# Quickstart

The simplest qhook setup. No Docker required -- just the binary and curl.

## Setup

1. Install qhook:

```bash
cargo install qhook
```

2. Start qhook with the example config:

```bash
qhook start -c examples/quickstart/qhook.yaml
```

3. In another terminal, start the mock receiver:

```bash
python3 examples/quickstart/receiver.py
```

4. Send a test event:

```bash
curl -X POST http://localhost:8888/events/app/order.created \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_001", "customer": "alice", "amount": 4999}'
```

## Expected Output

The receiver logs:

```
[order] received: id=ord_001, customer=alice, amount=4999
```

qhook logs:

```
event_id=01JXXXX event_type=order.created source=app
job delivered handler=process-order status=200
```

## What This Shows

- **Event ingestion** via `POST /events/{source}/{event_type}`
- **Automatic queuing** with retry on failure
- **Delivery** to your HTTP handler with the original JSON payload

## Next Steps

- Add signature verification: see [github-webhook](../github-webhook/)
- Add filtering and transformation: see [filter-transform](../filter-transform/)
- Add Stripe checkout handling: see [stripe-checkout](../stripe-checkout/)
