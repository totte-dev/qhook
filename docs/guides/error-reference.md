---
layout: default
title: Error Reference
---

# Error Reference

This page documents all HTTP status codes and error messages returned by qhook endpoints.

## Status Codes

| Code | Meaning | When |
|------|---------|------|
| 200  | OK | Webhook/SNS processed successfully |
| 202  | Accepted | Event accepted via `/events` |
| 400  | Bad Request | Invalid payload or message format |
| 401  | Unauthorized | Signature verification or auth token failed |
| 404  | Not Found | Unknown source or invalid callback token |
| 413  | Payload Too Large | Request body exceeds `max_body_size` (default 1MB) |
| 429  | Too Many Requests | Per-IP rate limit exceeded |
| 500  | Internal Server Error | Database or verification error |
| 503  | Service Unavailable | Concurrency limit exceeded or database unreachable |

## Error Messages by Endpoint

### POST /webhooks/{source}

| Status | Message | Cause |
|--------|---------|-------|
| 200 | `Event received` | Event processed and jobs created |
| 200 | `Duplicate event ignored` | Idempotency key already exists |
| 400 | `Invalid UTF-8` | Request body is not valid UTF-8 |
| 400 | `Invalid signature` | Signature present but format is invalid |
| 401 | `Invalid signature` | Signature verification failed |
| 404 | `Unknown source` | Source name not in config or wrong type |
| 500 | `Verification error` | Signature verification encountered an internal error |
| 500 | `Internal error` | Database write failed |

### POST /events/{source}/{event_type}

| Status | Message | Cause |
|--------|---------|-------|
| 202 | `Event accepted` | Event queued for delivery |
| 400 | `Invalid UTF-8` | Request body is not valid UTF-8 |
| 401 | `Invalid token` | Missing or invalid `Authorization: Bearer <token>` |
| 500 | `Internal error` | Database write failed |

### POST /sns/{source}

| Status | Message | Cause |
|--------|---------|-------|
| 200 | `Event received` | SNS notification processed |
| 200 | `Duplicate event ignored` | Event with same key already exists |
| 200 | `Subscription confirmed` | SNS subscription auto-confirmed |
| 200 | `Unsubscribe acknowledged` | Unsubscribe confirmation logged |
| 400 | `Invalid UTF-8` | Request body is not valid UTF-8 |
| 400 | `Invalid SNS message` | JSON does not contain required SNS fields |
| 400 | `Unknown message type` | SNS `Type` field is not recognized |
| 400 | `Invalid SubscribeURL` | URL points to non-HTTPS or non-AWS domain |
| 400 | `Missing SubscribeURL` | Subscription confirmation missing URL |
| 401 | `Invalid signature` | SNS X.509 signature verification failed |
| 404 | `Unknown source` | Source name not in config or not type `sns` |
| 500 | `Verification error` | Certificate fetch or verification error |
| 500 | `Subscription confirmation failed` | HTTP GET to SubscribeURL failed |
| 500 | `Internal error` | Database write failed |

### POST /callback/{token}

| Status | Response | Cause |
|--------|----------|-------|
| 200 | `{"status":"ok","message":"callback received"}` | Workflow step resumed |
| 404 | `{"error":"not found"}` | Token invalid, expired, or already used |
| 500 | `{"error":"internal error"}` | Database error during callback processing |

> **Note:** All failure cases return 404 to prevent token enumeration.

### GET /health

| Status | Response | Cause |
|--------|----------|-------|
| 200 | `{"status":"ok","queue_depth":N}` | Service healthy |
| 503 | `{"status":"error","detail":"database unreachable"}` | Database connection failed |

### GET /metrics

| Status | Response | Cause |
|--------|----------|-------|
| 200 | Prometheus text format | Metrics returned |
| 401 | `Invalid token` | Missing or invalid `Authorization: Bearer <token>` (when `metrics_auth_token` configured) |

## Global Middleware Errors

These can be returned by any endpoint:

| Status | Cause | Configuration |
|--------|-------|---------------|
| 413 | Request body exceeds size limit | `server.max_body_size` (default: 1MB) |
| 429 | Per-IP rate limit exceeded | `server.ip_rate_limit` |
| 503 | Inbound concurrency limit exceeded | `server.max_inbound` (default: 100) |

## Security Headers

All responses include:

```
x-content-type-options: nosniff
x-frame-options: DENY
cache-control: no-store
```
