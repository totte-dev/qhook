---
layout: default
title: Filtering & Transformation
---

# Filtering & Transformation

## Event Filtering

Use `filter` on a handler to only create jobs when the payload matches a condition. Events that don't match are silently skipped -- no job is created.

```yaml
handlers:
  paid-only:
    source: stripe
    events: [invoice.*]
    url: http://backend:3000/paid
    filter: "$.data.object.status == paid"
```

### Filter Expressions

| Syntax | Example | Description |
|--------|---------|-------------|
| `$.path == value` | `$.status == active` | Equality (strings, numbers, booleans) |
| `$.path != value` | `$.env != test` | Inequality |
| `$.path >= value` | `$.amount >= 1000` | Greater than or equal (numeric) |
| `$.path > value` | `$.retries > 0` | Greater than (numeric) |
| `$.path <= value` | `$.priority <= 5` | Less than or equal (numeric) |
| `$.path < value` | `$.age < 18` | Less than (numeric) |
| `$.path in [a, b]` | `$.type in [created, updated]` | Set membership |
| `$.path` | `$.data.verified` | Truthy (exists, not null/false/0/"") |

### Examples

```yaml
# Only process paid invoices
filter: "$.data.object.status == paid"

# Exclude test events
filter: "$.environment != test"

# Multiple event types
filter: "$.action in [opened, synchronize, reopened]"

# Check if a field is truthy (exists and not null/false/0)
filter: "$.data.customer.verified"

# Nested paths
filter: "$.data.object.customer.email == user@example.com"
```

## Payload Transformation

Use `transform` to reshape the payload before delivery. The original payload is preserved in the database; transformation is applied at delivery time.

```yaml
handlers:
  slack-notify:
    source: stripe
    events: [checkout.session.completed]
    url: http://slack-bot:3000/notify
    transform: |
      {"text": "New order from {{$.data.object.customer_email}} for {{$.data.object.amount_total}} cents"}
```

### Placeholder Syntax

- `{{$.path}}` -- replaced with the value at the given JSONPath
- Missing fields resolve to `null`
- String values are JSON-escaped (safe from injection)
- Supports strings, numbers, booleans, arrays, and nested objects

### Examples

**Reshape for a downstream API:**

```yaml
transform: |
  {"order_id": "{{$.id}}", "customer": "{{$.customer}}", "total": {{$.amount}}}
```

Input: `{"id": "ord_123", "customer": "alice", "amount": 4999, "currency": "jpy"}`

Output: `{"order_id": "ord_123", "customer": "alice", "total": 4999}`

**Slack notification format:**

```yaml
transform: |
  {"text": "Order {{$.id}} from {{$.customer}}: {{$.amount}} {{$.currency}}"}
```

Output: `{"text": "Order ord_123 from alice: 4999 jpy"}`

## Combining Filter and Transform

Filter is evaluated first. If it matches, the transform is applied before delivery.

```yaml
handlers:
  audit-paid:
    source: stripe
    events: [invoice.*]
    url: http://audit:3000/log
    filter: "$.status == paid"
    transform: |
      {"invoice_id": "{{$.id}}", "amount": {{$.amount}}, "paid": true}
```

Only paid invoices are delivered, and they arrive in a compact audit format.

## See Also

- [filter-transform example](https://github.com/totte-dev/qhook/tree/main/examples/filter-transform) -- working example with three handler patterns
