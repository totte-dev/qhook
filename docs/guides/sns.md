---
layout: default
title: AWS SNS
---

# AWS SNS

qhook can receive events from AWS SNS topics. It handles the full lifecycle automatically.

## How It Works

1. **Subscription confirmation** -- when SNS sends a `SubscriptionConfirmation` message, qhook automatically confirms by fetching the `SubscribeURL`.
2. **Message unwrapping** -- SNS wraps your payload in an envelope. qhook extracts the `Message` field and delivers only the actual payload to your handlers.
3. **Signature verification** -- each SNS message is verified using the X.509 certificate from AWS (SHA1/SHA256). Certificates are cached for 1 hour.

## Setup

```yaml
sources:
  my-sns:
    type: sns
```

Point your SNS subscription to:

```
https://your-qhook-host/sns/my-sns
```

The event type is extracted from:
1. The message payload `type` field
2. The `detail-type` field (for EventBridge events via SNS)
3. The SNS `Subject` field

## Handler Example

```yaml
sources:
  my-sns:
    type: sns

handlers:
  process-notification:
    source: my-sns
    events: [order.created]
    url: http://backend:3000/jobs/sns-order
```

## Testing with LocalStack

For local development, use `skip_verify: true` to bypass X.509 signature verification:

```yaml
sources:
  my-sns:
    type: sns
    skip_verify: true     # LocalStack does not sign messages
```

Run LocalStack:

```bash
docker run -d --name localstack -p 4566:4566 -e SERVICES=sns localstack/localstack:3
```

Create a topic and subscription:

```bash
aws --endpoint-url=http://localhost:4566 sns create-topic --name my-topic
aws --endpoint-url=http://localhost:4566 sns subscribe \
  --topic-arn arn:aws:sns:us-east-1:000000000000:my-topic \
  --protocol http \
  --notification-endpoint http://host.docker.internal:8888/sns/my-sns
```

Publish a test message:

```bash
aws --endpoint-url=http://localhost:4566 sns publish \
  --topic-arn arn:aws:sns:us-east-1:000000000000:my-topic \
  --message '{"type":"order.created","id":"ord_123","amount":4999}'
```

## Security

- **SubscribeURL validation**: qhook validates that the `SubscribeURL` points to a legitimate SNS domain before fetching, preventing SSRF attacks.
- **Certificate caching**: Signing certificates are cached for 1 hour to prevent DoS via repeated certificate fetches.
- **Certificate URL validation**: Only certificates hosted on `sns.*.amazonaws.com` domains are accepted.
