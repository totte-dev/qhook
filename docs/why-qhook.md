# Why qhook?

Receiving a webhook is trivial -- a few lines of code. But running webhooks **safely in production** is a different story. This document shows the concrete difference in code volume and operational burden between a DIY implementation and qhook.

---

## 1. The Problem with Webhooks

When you receive webhooks directly from external services (Stripe, GitHub, Shopify, etc.), you have to handle all of the following:

- **Signature verification per provider** -- Stripe uses `t=...` HMAC-SHA256, GitHub uses `sha256=...`, Shopify uses Base64 HMAC. Each is different.
- **Retry with backoff** -- When your downstream service is down, you need exponential backoff and eventually a Dead Letter Queue (DLQ).
- **Idempotency** -- Providers retry, so the same event arrives multiple times. Without dedup, you get double charges or duplicate processing.
- **Event loss during outages** -- Webhooks that arrive while your app is down are gone forever unless you persist them to a queue.
- **Fan-out to multiple services** -- Routing one webhook to multiple microservices requires dispatch logic.

All of these are **not your business logic**, yet they cost real engineering time to implement, test, and maintain.

---

## 2. Before: Without qhook

Handling Stripe webhooks directly in Python/Flask, with signature verification, idempotency, and async retry:

```python
import hashlib
import hmac
import json
import time
from functools import wraps

from flask import Flask, request, abort, jsonify
from celery import Celery
from sqlalchemy import create_engine, Column, String, DateTime
from sqlalchemy.orm import sessionmaker, declarative_base

app = Flask(__name__)
engine = create_engine("postgresql://localhost/myapp")
Session = sessionmaker(bind=engine)
Base = declarative_base()
celery = Celery("tasks", broker="redis://localhost:6379/0")

STRIPE_WEBHOOK_SECRET = "whsec_..."
GITHUB_WEBHOOK_SECRET = "gh_secret_..."


# --- Idempotency table ---
class ProcessedEvent(Base):
    __tablename__ = "processed_events"
    idempotency_key = Column(String, primary_key=True)
    source = Column(String, nullable=False)
    created_at = Column(DateTime, server_default="now()")


# --- Signature verification (Stripe) ---
def verify_stripe_signature(payload: bytes, sig_header: str, secret: str) -> bool:
    pairs = dict(p.split("=", 1) for p in sig_header.split(",") if "=" in p)
    timestamp = pairs.get("t", "")
    signature = pairs.get("v1", "")
    if not timestamp or not signature:
        return False
    signed = f"{timestamp}.{payload.decode()}".encode()
    expected = hmac.new(secret.encode(), signed, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, signature)


# --- Signature verification (GitHub) ---
def verify_github_signature(payload: bytes, sig_header: str, secret: str) -> bool:
    if not sig_header.startswith("sha256="):
        return False
    expected = hmac.new(secret.encode(), payload, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, sig_header[7:])


# --- Async task with retry ---
@celery.task(bind=True, max_retries=5, default_retry_delay=30)
def process_stripe_event(self, event_data: dict):
    try:
        event_type = event_data.get("type", "")
        if event_type == "invoice.paid":
            handle_invoice_paid(event_data)
        elif event_type == "checkout.session.completed":
            handle_checkout_completed(event_data)
        # ... more event type branches ...
    except Exception as exc:
        raise self.retry(exc=exc, countdown=30 * (2 ** self.request.retries))


@celery.task(bind=True, max_retries=5, default_retry_delay=30)
def process_github_event(self, event_data: dict):
    try:
        handle_github_push(event_data)
    except Exception as exc:
        raise self.retry(exc=exc, countdown=30 * (2 ** self.request.retries))


# --- Endpoint (Stripe) ---
@app.route("/webhooks/stripe", methods=["POST"])
def stripe_webhook():
    payload = request.get_data()
    sig = request.headers.get("Stripe-Signature", "")

    if not verify_stripe_signature(payload, sig, STRIPE_WEBHOOK_SECRET):
        abort(401)

    data = json.loads(payload)
    idempotency_key = data.get("id", "")

    session = Session()
    if session.query(ProcessedEvent).get(idempotency_key):
        return jsonify({"status": "duplicate"}), 200

    session.add(ProcessedEvent(
        idempotency_key=idempotency_key, source="stripe"
    ))
    session.commit()

    process_stripe_event.delay(data)
    return jsonify({"status": "accepted"}), 200


# --- Endpoint (GitHub) --- nearly identical boilerplate
@app.route("/webhooks/github", methods=["POST"])
def github_webhook():
    payload = request.get_data()
    sig = request.headers.get("X-Hub-Signature-256", "")

    if not verify_github_signature(payload, sig, GITHUB_WEBHOOK_SECRET):
        abort(401)

    data = json.loads(payload)

    session = Session()
    delivery_id = request.headers.get("X-GitHub-Delivery", "")
    if session.query(ProcessedEvent).get(delivery_id):
        return jsonify({"status": "duplicate"}), 200

    session.add(ProcessedEvent(
        idempotency_key=delivery_id, source="github"
    ))
    session.commit()

    process_github_event.delay(data)
    return jsonify({"status": "accepted"}), 200


def handle_invoice_paid(event):
    ...  # business logic

def handle_checkout_completed(event):
    ...  # business logic

def handle_github_push(event):
    ...  # business logic
```

**~90 lines.** The only business logic is the `handle_*` functions at the bottom -- everything else is infrastructure boilerplate. On top of that:

- Celery + Redis (or RabbitMQ) infrastructure required separately
- PostgreSQL migration for the events table
- DLQ monitoring / retry UI needs Flower or custom tooling
- Every new provider means another endpoint + signature verification

---

## 3. After: With qhook

### qhook.yaml (config)

```yaml
sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}
  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}

delivery:
  default_retry:
    max: 5
    backoff: exponential
    interval: 30s

handlers:
  payment:
    source: stripe
    events: [invoice.paid, checkout.session.completed]
    url: http://localhost:3000/jobs/payment
    idempotency_key: "$.id"
  repo-push:
    source: github
    events: ["*"]
    url: http://localhost:3000/jobs/github
    idempotency_key: "$.head_commit.id"
```

### Application code (receiver)

```python
from flask import Flask, request, jsonify

app = Flask(__name__)


@app.route("/jobs/payment", methods=["POST"])
def handle_payment():
    event = request.get_json()
    # Just write your business logic
    if event["type"] == "invoice.paid":
        activate_subscription(event["data"]["object"])
    elif event["type"] == "checkout.session.completed":
        send_receipt(event["data"]["object"])
    return jsonify({"ok": True})


@app.route("/jobs/github", methods=["POST"])
def handle_github():
    event = request.get_json()
    trigger_ci_pipeline(event)
    return jsonify({"ok": True})
```

**20 lines of config + 20 lines of code.** The app side is pure business logic.

Signature verification, idempotency, retry, DLQ -- all handled by qhook. Return HTTP 200 and it's done; return 5xx and qhook retries with exponential backoff.

---

## 4. Comparison

| | DIY | qhook |
|---|---|---|
| **Code** | ~90 lines (infra only) | 20 lines config + 20 lines logic |
| **Signature verification** | Implement per provider | `verify: stripe` -- one line |
| **Idempotency** | DB table + dedup code | `idempotency_key: "$.id"` |
| **Retry** | Celery + Redis required | Built-in (exponential backoff) |
| **DLQ** | Design & build yourself | Built-in (`qhook jobs list --status dead`) |
| **Event persistence** | Design your own DB schema | Auto-saved to SQLite/Postgres |
| **Fan-out** | Manual dispatch logic | Define multiple handlers |
| **Adding providers** | New endpoint + verification | Add a few lines to sources |
| **AWS SNS** | Parse envelope, verify X.509, confirm subscription | `type: sns` -- one line |
| **CloudEvents** | Parse headers/envelope, forward metadata | Automatic detection |
| **External deps** | Redis/RabbitMQ, Celery, etc. | Single qhook binary |
| **Monitoring** | Flower or custom UI | `qhook jobs` / `qhook events` CLI |
| **Recovery** | Custom recovery procedures | `qhook jobs retry` |

---

## 5. Where qhook Shines

### Multiple providers at once

When receiving Stripe + GitHub + Shopify in one service, a DIY approach requires three different signature verification implementations. With qhook:

```yaml
sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}
  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}
  shopify:
    type: webhook
    verify: shopify
    secret: ${SHOPIFY_WEBHOOK_SECRET}
```

qhook absorbs the algorithm differences (Stripe's `t=...` format, GitHub's `sha256=...` format, Shopify's Base64 HMAC). Your app only receives verified payloads.

### AWS SNS as an event source

Receiving events from SNS requires handling subscription confirmation, envelope unwrapping, and X.509 certificate verification. With qhook, it's one line:

```yaml
sources:
  notifications:
    type: sns
```

qhook automatically confirms subscriptions, verifies message signatures, unwraps the SNS envelope, and delivers the actual message payload to your handlers.

### Fan-out to multiple services

Deliver a single Stripe event to billing, notification, and analytics services simultaneously:

```yaml
handlers:
  billing:
    source: stripe
    events: [invoice.paid]
    url: http://billing-service:3000/webhook
    idempotency_key: "$.id"
  notification:
    source: stripe
    events: [invoice.paid, invoice.payment_failed]
    url: http://notification-service:3000/webhook
    idempotency_key: "$.id"
  analytics:
    source: stripe
    events: ["*"]
    url: http://analytics-service:3000/ingest
```

With DIY, you'd dispatch async tasks per destination inside your endpoint, each with its own retry logic. qhook creates independent jobs per handler with individual retry and DLQ management.

### Production retry & DLQ

During development, receiving and processing webhooks seems simple enough. But in production:

- Webhooks arrive during deployments
- Transient DB failures cause processing errors
- External API rate limits trigger failures

Without a persistent queue + retry, these transient failures mean lost events. qhook provides this near-zero-config:

```bash
# Check dead jobs
qhook jobs list --status dead

# Retry a specific job
qhook jobs retry <job-id>

# Retry all dead jobs
qhook jobs retry
```

---

## Summary

qhook solves the **webhook infrastructure layer**. Signature verification, persistence, idempotency, retry, DLQ, fan-out routing -- all separated from your application code and managed declaratively via config.

Your app receives verified, deduplicated, reliably-delivered payloads and focuses on business logic alone.
