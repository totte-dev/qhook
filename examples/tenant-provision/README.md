# Tenant Provisioning Example

Multi-step tenant provisioning with input validation, authenticated API calls, and reverse-order rollback on failure.

## What it demonstrates

- **Workflow input params** — validates `tenant_id` (string, required), `region` (string, required), `plan` (string, optional) before starting
- **Custom headers** — sends `Authorization: Bearer ...` to infrastructure API
- **Reverse rollback** — on failure, cleans up in reverse order (DB → infra → notify)
- **Input templating** — passes `tenant_id` and `region` from the event payload to each step

## Pipeline

```
tenant.create event
  → create-infra (with auth header)
  → create-db
  → configure
  → activate
  ↓ failure at any step
  → rollback-db → rollback-infra → notify-failure
```

## Run

```bash
# Start qhook (mock server not included — replace URLs with your services)
qhook start -c qhook.yaml

# Send a tenant.create event
curl -X POST http://localhost:8888/events/tenant.create \
  -H "Authorization: Bearer test-token" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id": "t-123", "region": "us-east-1", "plan": "pro"}'

# Missing required param → rejected
curl -X POST http://localhost:8888/events/tenant.create \
  -H "Authorization: Bearer test-token" \
  -H "Content-Type: application/json" \
  -d '{"plan": "pro"}'
```
