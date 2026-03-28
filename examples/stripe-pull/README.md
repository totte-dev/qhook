# Stripe Pull-Mode Example

Consume Stripe webhooks via pull-mode queues instead of push delivery. Your consumer polls for messages at its own pace -- no need to run an HTTP server.

## Architecture

```
Stripe  --->  qhook (verify + queue)  <---  consumer (poll + ack)
                                              |
                                              +-- process locally
                                              +-- ack on success
                                              +-- nack on failure (retry)
```

Unlike the [stripe-checkout](../stripe-checkout/) example where qhook pushes to your HTTP handler, here your consumer **pulls** messages from a queue. This is useful when:

- Your consumer is a script, CLI tool, or serverless function
- You want to control concurrency on the consumer side
- You don't want to run an HTTP server

## Setup

1. Start qhook:

```bash
qhook start -c examples/stripe-pull/qhook.yaml
```

2. Run the consumer (pick one):

```bash
# Python (no dependencies)
python3 examples/stripe-pull/consumer.py

# TypeScript (Node.js 18+)
npx tsx examples/stripe-pull/consumer.ts
```

## Testing with curl

Send a test event directly (bypasses Stripe signature verification):

```bash
# checkout.session.completed
curl -X POST http://localhost:8888/events/stripe/checkout.session.completed \
  -H "Content-Type: application/json" \
  -d '{
    "id": "cs_test_abc123",
    "object": "checkout.session",
    "amount_total": 4999,
    "currency": "jpy",
    "customer": "cus_xyz789",
    "payment_status": "paid"
  }'

# charge.failed
curl -X POST http://localhost:8888/events/stripe/charge.failed \
  -H "Content-Type: application/json" \
  -d '{
    "id": "ch_test_fail456",
    "failure_message": "Your card was declined"
  }'
```

## Testing with Stripe CLI

```bash
export STRIPE_WEBHOOK_SECRET=whsec_...
stripe listen --forward-to localhost:8888/webhooks/stripe
stripe trigger checkout.session.completed
```

## Expected Output

The consumer logs:

```
Polling queue 'payments' at http://localhost:8888 (Ctrl+C to stop)
[payment] completed: id=cs_test_abc123, amount=4999, customer=cus_xyz789
  acked 1 message(s)
[charge] failed: id=ch_test_fail456, failure=Your card was declined
  acked 1 message(s)
```

## Queue API Reference

| Endpoint | Method | Description |
|---|---|---|
| `/api/queues/payments/messages?wait=10s&batch=5` | GET | Long-poll for messages |
| `/api/queues/payments/ack` | POST | Acknowledge processed messages |
| `/api/queues/payments/nack` | POST | Reject messages (retry or DLQ) |

## Next Steps

- Add `api_key` to the queue config for authentication in production
- Adjust `visibility_timeout` and `max_attempts` for your workload
- See [stripe-checkout](../stripe-checkout/) for the push-mode alternative
