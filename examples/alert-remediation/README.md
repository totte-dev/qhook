# Alert Remediation Example

PagerDuty alert triggers automatic remediation based on severity, with escalation to PagerDuty API if remediation fails.

## What it demonstrates

- **PagerDuty webhook verification** — `verify: pagerduty` validates HMAC-SHA256 signature
- **Choice step** — routes to different actions based on `$.severity`
- **Wait step** — pauses 30 seconds after remediation for recovery
- **Catch → escalate** — if health check fails after remediation, escalates via PagerDuty API
- **Custom headers** — authenticates to PagerDuty API with `Authorization` header

## Pipeline

```
PagerDuty incident.triggered
  → triage (choice by severity)
    critical → restart → wait 30s → health-check
    warning  → scale-up → wait 30s → health-check
    default  → notify
  ↓ health-check fails
  → escalate (PagerDuty API with auth)
```

## Run

```bash
# Start qhook
PAGERDUTY_WEBHOOK_SECRET=my-secret qhook start -c qhook.yaml

# Simulate a PagerDuty webhook (compute HMAC for your secret)
SECRET=my-secret
PAYLOAD='{"event":{"event_type":"incident.triggered"},"severity":"critical"}'
SIG=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$SECRET" | cut -d' ' -f2)

curl -X POST http://localhost:8888/webhooks/pagerduty \
  -H "Content-Type: application/json" \
  -H "X-PagerDuty-Signature: v1=$SIG" \
  -d "$PAYLOAD"
```
