# Stripe Checkout Example

Process Stripe `checkout.session.completed` webhooks through qhook.

## Architecture

```
Stripe  --->  qhook (verify + queue)  --->  Flask app
                  |                             |
                  +-- /jobs/payment      payment confirmation
                  +-- /jobs/fulfillment  shipping / fulfillment
```

A single webhook event is delivered to **two handlers** (payment and fulfillment) simultaneously, each with independent retry settings and idempotency keys.

## Getting Started

```bash
docker compose up
```

qhook runs on `localhost:8888`, and the Flask app runs internally at `app:5000`.

## Testing

Send a test event directly (bypassing signature verification):

```bash
curl -X POST http://localhost:8888/events/checkout.session.completed \
  -H "Content-Type: application/json" \
  -d '{
    "id": "cs_test_abc123",
    "object": "checkout.session",
    "amount_total": 4999,
    "currency": "jpy",
    "customer": "cus_xyz789",
    "payment_status": "paid"
  }'
```

> **Note:** This uses the `/events/` endpoint (internal event API).
> In production, Stripe POSTs to `/webhooks/stripe` and signature verification is enforced.

## Expected Output

The Flask app logs should show:

```
[payment] event received: id=cs_test_abc123, amount=4999
[fulfillment] started: id=cs_test_abc123, customer=cus_xyz789
```
