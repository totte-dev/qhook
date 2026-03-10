---
layout: default
title: Kubernetes (Helm)
---

# Kubernetes (Helm)

Deploy qhook to Kubernetes using the included Helm chart.

## Prerequisites

- Kubernetes 1.24+
- Helm 3.x
- `kubectl` configured for your cluster

## Quick Start

```bash
# From the qhook repository root
helm install qhook ./charts/qhook
```

## Configuration

Override values via `--set` or a custom values file:

```bash
helm install qhook ./charts/qhook -f my-values.yaml
```

### Inline Config

Provide qhook config directly in values:

```yaml
# my-values.yaml
config:
  database:
    driver: sqlite
  sources:
    github:
      type: webhook
      verify: github
      secret: "${GITHUB_WEBHOOK_SECRET}"
  handlers:
    deploy:
      source: github
      events: [push]
      url: http://backend:3000/deploy
```

### External ConfigMap

Use an existing ConfigMap instead of inline config:

```yaml
existingConfigMap: my-qhook-config
```

### Secrets

Pass sensitive values (database URL, webhook secrets) via a Kubernetes Secret:

```bash
kubectl create secret generic qhook-secrets \
  --from-literal=DATABASE_URL=postgres://user:pass@db:5432/qhook \
  --from-literal=GITHUB_WEBHOOK_SECRET=whsec_xxx
```

```yaml
existingSecret: qhook-secrets
```

## Persistence (SQLite)

For SQLite, enable a PersistentVolumeClaim:

```yaml
persistence:
  enabled: true
  storageClass: gp3
  size: 5Gi

config:
  database:
    driver: sqlite
```

> Note: SQLite with PVC limits you to a single replica. Use Postgres for multi-replica deployments.

## Postgres

For production multi-instance deployments:

```yaml
replicaCount: 3

config:
  database:
    driver: postgres

existingSecret: qhook-db-credentials  # must contain DATABASE_URL

persistence:
  enabled: false
```

## Ingress

```yaml
ingress:
  enabled: true
  className: nginx
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
  hosts:
    - host: webhooks.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: qhook-tls
      hosts:
        - webhooks.example.com
```

## Autoscaling

```yaml
autoscaling:
  enabled: true
  minReplicas: 2
  maxReplicas: 10
  targetCPUUtilizationPercentage: 70

resources:
  requests:
    cpu: 100m
    memory: 64Mi
  limits:
    cpu: 500m
    memory: 256Mi
```

> Autoscaling requires Postgres (SQLite is single-writer).

## Health Checks

The chart configures liveness and readiness probes pointing to `/healthz`:

- **Liveness**: checks every 15s after 5s initial delay
- **Readiness**: checks every 10s after 3s initial delay

## Upgrading

```bash
helm upgrade qhook ./charts/qhook -f my-values.yaml
```

Config changes trigger a rolling restart automatically (via config checksum annotation).

## Uninstalling

```bash
helm uninstall qhook
```

> PVCs are not deleted automatically. Remove manually if no longer needed.
