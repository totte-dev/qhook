---
layout: default
title: Scaling Guide
---

# Scaling Guide: Choosing the Right Architecture

qhook supports four database drivers — SQLite, D1, PostgreSQL, and MySQL.
This guide helps you choose the right one based on your webhook volume, reliability requirements, and budget.

## Quick Decision Table

| Monthly webhooks | Recommended setup | Est. cost | Queue reliability |
|---|---|---|---|
| < 10K | **Cloudflare Containers + D1** | $0 (free tier) | Good for low concurrency |
| 10K - 500K | **Cloudflare Containers + D1** | $5 | Good |
| 500K - 5M | **Cloudflare Containers + D1** or **VPS + Postgres** | $5-10 | Good (CF) / Excellent (PG) |
| 5M - 50M | **VPS + PostgreSQL** | $10-30 | Excellent |
| 50M+ | **Multi-instance + PostgreSQL** | $50-200 | Excellent |

---

## Tier 1: Cloudflare Containers + D1 (0 - 5M webhooks/month)

**Best for**: Side projects, startups, low-to-medium traffic services.

```yaml
database:
  driver: d1
  account_id: ${CF_ACCOUNT_ID}
  database_id: ${CF_D1_DATABASE_ID}
  api_token: ${CF_API_TOKEN}
```

### Why it works

- **Scale to zero**: No traffic = no cost. Container sleeps after idle timeout.
- **Global edge**: Runs close to webhook sources (Stripe, GitHub, etc.).
- **No infrastructure to manage**: `wrangler deploy` and done.

### Cost breakdown

| Component | Free tier | Workers Standard ($5/mo) |
|---|---|---|
| CPU | 10 vCPU-hr/mo | 375 vCPU-min + $0.00002/vCPU-sec |
| D1 reads | 5M rows/day | 25B rows/mo included |
| D1 writes | 100K rows/day | 50M rows/mo included |
| Storage | 5 GB | 5 GB included |
| Egress | Unlimited | Unlimited |

**Example**: 500K webhooks/month = 83 CPU-min + 1.5M D1 writes → **$5/month**.

### D1 performance limits

D1 is SQLite-based with specific constraints that matter for a webhook queue:

| Constraint | Limit | Impact on qhook |
|---|---|---|
| **Write throughput** | 500-2,000 rows/sec | ~150-600 webhooks/sec (each = 2-3 writes) |
| **Concurrency** | Single writer | Writes are serialized; concurrent webhooks queue up |
| **Transactions** | Batch only (no BEGIN/COMMIT) | `drain` command is not atomic |
| **Request queue** | Limited depth | Returns "overloaded" error when exceeded |
| **Max DB size** | 10 GB | ~10-50M events depending on payload size |

### When D1 is fine

- Webhooks arrive at < 100/sec sustained (most SaaS integrations)
- Single consumer for pull-mode queues
- Payload sizes under 10 KB
- You value zero-ops over maximum throughput

### When D1 becomes a problem

- Sustained bursts > 200 webhooks/sec (e.g., bulk Shopify order imports)
- Multiple concurrent pull-mode consumers competing for messages
- `FOR UPDATE SKIP LOCKED` not available — optimistic locking only, more contention
- Large fan-out (1 event → 10+ handlers) multiplies write pressure
- D1 "overloaded" errors during traffic spikes

### Monitoring the boundary

Watch these signals to know when to migrate:

```bash
# Check D1 write usage (Cloudflare dashboard or API)
# qhook metrics endpoint
curl http://localhost:8888/metrics | grep qhook_deliveries

# If you see these in logs, it's time to migrate:
# - "D1 overloaded" errors
# - Delivery latency > 5s
# - Stale job recovery running frequently
```

---

## Tier 2: VPS + PostgreSQL (5M+ webhooks/month)

**Best for**: Growing services, production workloads, high reliability requirements.

```yaml
database:
  driver: postgres
  url: ${DATABASE_URL}
```

### Why Postgres

| Feature | D1 | PostgreSQL |
|---|---|---|
| Write throughput | 500-2K rows/sec | 10K-100K+ rows/sec |
| Concurrent writers | Single | Unlimited (MVCC) |
| `FOR UPDATE SKIP LOCKED` | Not available | Yes — lock-free job polling |
| Transactions | Batch only | Full ACID |
| Multi-instance qhook | Not safe | Safe (skip-locked) |
| Max DB size | 10 GB | Unlimited |

The key advantage: **`FOR UPDATE SKIP LOCKED`** allows multiple qhook instances to poll jobs without conflicts. D1/SQLite uses optimistic locking where consumers can contend.

### Recommended hosting

| Provider | Spec | Cost | Notes |
|---|---|---|---|
| **Hetzner CX22** | 2 vCPU, 4 GB | €4.35/mo | Best value. Add managed PG (€4/mo) |
| **Fly.io** | shared-cpu-1x | $1.94/mo | Postgres via Supabase/Neon |
| **Railway** | Usage-based | ~$5-10/mo | Managed Postgres included |
| **Render** | Starter | $7/mo | Managed Postgres $7/mo |

**Example**: 10M webhooks/month on Hetzner = **€8.35/month** (VPS + Postgres).

### Migration from D1

qhook handles this with a config change only — no data migration tool needed:

```yaml
# Before (D1)
database:
  driver: d1
  account_id: ...

# After (Postgres)
database:
  driver: postgres
  url: postgres://user:pass@host:5432/qhook
```

qhook auto-creates tables on first start. Historical events in D1 are not migrated (export with `qhook export events` first if needed).

### Multi-instance deployment

At high volume, run multiple qhook instances behind a load balancer:

```
                    ┌── qhook instance 1 ──┐
LB → webhooks ──→  ├── qhook instance 2 ──├──→ PostgreSQL
                    └── qhook instance 3 ──┘
```

Each instance polls independently using `FOR UPDATE SKIP LOCKED` — no job is processed twice.

```yaml
# Tune for multi-instance
worker:
  max_concurrency: 20
  batch_size: 20
```

---

## Tier 3: When You Need More (Cloud version)

At enterprise scale (50M+ webhooks/month), self-hosting qhook still works but operational burden grows:

- Database maintenance (vacuuming, index bloat, replication)
- Monitoring and alerting infrastructure
- Multi-region deployment for latency
- Compliance and audit requirements

This is where a managed Cloud version would provide value beyond raw infrastructure:

| Capability | Self-hosted | Cloud version |
|---|---|---|
| Multi-region | Manual setup | Built-in |
| Auto-scaling | Manual | Automatic |
| Dashboard + analytics | Prometheus + Grafana | Included |
| Compliance reporting | DIY | Built-in audit trail |
| Support | Community | Dedicated |

---

## Summary: Cost vs. Volume

```
Cost/month
  $200 ┤
       │                                          ╱ Multi-instance
  $100 ┤                                    ╱───── Postgres
       │                              ╱────╱
   $50 ┤                        ╱────╱
       │               ╱──────╱
   $10 ┤         ╱────╱
       │   ╱────╱ D1 write costs
    $5 ┤──╱────────── Cloudflare + D1
       │ Free
    $0 ┼──────┬───────┬───────┬───────┬──────
         10K  100K    1M     10M    100M
                    webhooks/month

  ──── Cloudflare + D1
  ──── VPS + Postgres
  Crossover at ~5M webhooks/month
```

## Migration Checklist

When moving between tiers:

- [ ] Export events if history is needed: `qhook export events > backup.jsonl`
- [ ] Update `database.driver` and connection settings in `qhook.yaml`
- [ ] Start qhook — tables are created automatically
- [ ] Replay recent events if needed: `qhook replay-local backup.jsonl`
- [ ] Update webhook endpoint URLs at providers (Stripe, GitHub, etc.)
- [ ] Verify with `qhook doctor`
