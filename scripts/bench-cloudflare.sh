#!/bin/bash
# bench-cloudflare.sh — Benchmark qhook on Cloudflare Containers + D1.
#
# Prerequisites:
#   - wrangler CLI configured with Cloudflare account
#   - k6 installed (https://k6.io/docs/get-started/installation/)
#   - jq installed
#   - A deployed qhook instance on Cloudflare Containers (or provide QHOOK_URL)
#
# Usage:
#   ./scripts/bench-cloudflare.sh [profile]
#
# Profiles:
#   smoke   — 10 VUs,  30s  (quick sanity check)
#   load    — 50 VUs,  60s  (normal load)
#   stress  — 200 VUs, 120s (stress test)
#
# Environment variables:
#   QHOOK_URL      — Base URL of qhook instance (required)
#   PROFILE        — Load profile override (smoke|load|stress)
#   RESULTS_DIR    — Directory for results (default: ./bench-results)
#   WEBHOOK_SECRET — Secret for Stripe signature verification (optional)
#   QUEUE_NAME     — Queue name for pull-mode tests (default: bench)
#   QUEUE_TOKEN    — Auth token for queue API (optional)
#   SKIP_INGEST    — Set to 1 to skip ingestion benchmark
#   SKIP_DELIVERY  — Set to 1 to skip delivery benchmark
#   SKIP_PULL      — Set to 1 to skip pull-mode benchmark
#   SKIP_D1        — Set to 1 to skip D1 persistence benchmark
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
PROFILE="${1:-${PROFILE:-load}}"
QHOOK_URL="${QHOOK_URL:?Set QHOOK_URL to the deployed qhook instance URL}"
RESULTS_DIR="${RESULTS_DIR:-$PROJECT_DIR/bench-results}"
QUEUE_NAME="${QUEUE_NAME:-bench}"
QUEUE_TOKEN="${QUEUE_TOKEN:-}"
WEBHOOK_SECRET="${WEBHOOK_SECRET:-}"
TIMESTAMP=$(date -u +%Y%m%dT%H%M%SZ)

# Validate profile
case "$PROFILE" in
  smoke)  VUS=10;  DURATION="30s";  RAMP_UP="5s";  RAMP_DOWN="5s"  ;;
  load)   VUS=50;  DURATION="60s";  RAMP_UP="10s"; RAMP_DOWN="5s"  ;;
  stress) VUS=200; DURATION="120s"; RAMP_UP="20s"; RAMP_DOWN="10s" ;;
  *)
    echo "Unknown profile: $PROFILE (use: smoke, load, stress)"
    exit 1
    ;;
esac

mkdir -p "$RESULTS_DIR"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }

check_dep() {
  if ! command -v "$1" &>/dev/null; then
    echo "ERROR: $1 is required but not installed."
    exit 1
  fi
}

check_dep k6
check_dep jq
check_dep curl

# Verify qhook is reachable
echo "=== qhook Cloudflare Benchmark ==="
echo "  URL:      $QHOOK_URL"
echo "  Profile:  $PROFILE (${VUS} VUs, ${DURATION})"
echo "  Results:  $RESULTS_DIR"
echo ""

echo -n "Checking health..."
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$QHOOK_URL/health" || echo "000")
if [ "$HTTP_CODE" != "200" ]; then
  echo " FAILED (HTTP $HTTP_CODE)"
  echo "Cannot reach $QHOOK_URL/health — is qhook deployed?"
  exit 1
fi
echo " OK"
echo ""

# Collect instance metadata
METADATA=$(cat <<EOF
{
  "timestamp": "$TIMESTAMP",
  "profile": "$PROFILE",
  "vus": $VUS,
  "duration": "$DURATION",
  "qhook_url": "$QHOOK_URL",
  "platform": "cloudflare-containers-d1"
}
EOF
)
echo "$METADATA" | jq . > "$RESULTS_DIR/metadata-$TIMESTAMP.json"

# ---------------------------------------------------------------------------
# Benchmark 1: Webhook Ingestion Throughput
# ---------------------------------------------------------------------------
if [ "${SKIP_INGEST:-0}" != "1" ]; then
  echo "--- 1. Webhook Ingestion Throughput (POST /webhooks/stripe) ---"
  echo ""

  INGEST_RESULT="$RESULTS_DIR/ingest-$TIMESTAMP.json"

  k6 run \
    --out json="$INGEST_RESULT" \
    --summary-export="$RESULTS_DIR/ingest-summary-$TIMESTAMP.json" \
    -e QHOOK_URL="$QHOOK_URL" \
    -e VUS="$VUS" \
    -e DURATION="$DURATION" \
    -e RAMP_UP="$RAMP_UP" \
    -e RAMP_DOWN="$RAMP_DOWN" \
    -e WEBHOOK_SECRET="$WEBHOOK_SECRET" \
    -e SCENARIO="ingest" \
    "$SCRIPT_DIR/bench-k6-cloudflare.js" || true

  echo ""
fi

# ---------------------------------------------------------------------------
# Benchmark 2: Push Delivery Latency
# ---------------------------------------------------------------------------
if [ "${SKIP_DELIVERY:-0}" != "1" ]; then
  echo "--- 2. Push Delivery Latency (webhook → handler) ---"
  echo ""
  echo "  Sends events and measures time until handler receives delivery."
  echo "  Requires a handler configured to POST delivery timestamps."
  echo ""

  DELIVERY_RESULT="$RESULTS_DIR/delivery-$TIMESTAMP.json"

  k6 run \
    --out json="$DELIVERY_RESULT" \
    --summary-export="$RESULTS_DIR/delivery-summary-$TIMESTAMP.json" \
    -e QHOOK_URL="$QHOOK_URL" \
    -e VUS="$VUS" \
    -e DURATION="$DURATION" \
    -e RAMP_UP="$RAMP_UP" \
    -e RAMP_DOWN="$RAMP_DOWN" \
    -e SCENARIO="delivery" \
    "$SCRIPT_DIR/bench-k6-cloudflare.js" || true

  echo ""
fi

# ---------------------------------------------------------------------------
# Benchmark 3: Pull-Mode Queue Throughput
# ---------------------------------------------------------------------------
if [ "${SKIP_PULL:-0}" != "1" ]; then
  echo "--- 3. Pull-Mode Queue Throughput (GET /messages + POST /ack) ---"
  echo ""

  PULL_RESULT="$RESULTS_DIR/pull-$TIMESTAMP.json"

  # First, seed the queue with events
  echo "  Seeding queue with events..."
  SEED_COUNT=$((VUS * 20))
  for i in $(seq 1 "$SEED_COUNT"); do
    curl -s -X POST "$QHOOK_URL/events/bench-source/bench.test" \
      -H "Content-Type: application/json" \
      -d "{\"id\":\"seed-$i\",\"ts\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}" \
      -o /dev/null &
    # Limit concurrent curl processes
    if (( i % 50 == 0 )); then
      wait
      echo "    seeded $i/$SEED_COUNT"
    fi
  done
  wait
  echo "  Seeded $SEED_COUNT events."
  echo ""

  # Run pull-mode benchmark
  k6 run \
    --out json="$PULL_RESULT" \
    --summary-export="$RESULTS_DIR/pull-summary-$TIMESTAMP.json" \
    -e QHOOK_URL="$QHOOK_URL" \
    -e QUEUE_NAME="$QUEUE_NAME" \
    -e QUEUE_TOKEN="$QUEUE_TOKEN" \
    -e VUS="$VUS" \
    -e DURATION="$DURATION" \
    -e RAMP_UP="$RAMP_UP" \
    -e RAMP_DOWN="$RAMP_DOWN" \
    "$SCRIPT_DIR/bench-k6-pull.js" || true

  echo ""
fi

# ---------------------------------------------------------------------------
# Benchmark 4: D1 Performance Under Load
# ---------------------------------------------------------------------------
if [ "${SKIP_D1:-0}" != "1" ]; then
  echo "--- 4. D1 Performance Under Load (event persistence + query) ---"
  echo ""

  D1_RESULT="$RESULTS_DIR/d1-$TIMESTAMP.json"

  k6 run \
    --out json="$D1_RESULT" \
    --summary-export="$RESULTS_DIR/d1-summary-$TIMESTAMP.json" \
    -e QHOOK_URL="$QHOOK_URL" \
    -e VUS="$VUS" \
    -e DURATION="$DURATION" \
    -e RAMP_UP="$RAMP_UP" \
    -e RAMP_DOWN="$RAMP_DOWN" \
    -e SCENARIO="d1" \
    "$SCRIPT_DIR/bench-k6-cloudflare.js" || true

  echo ""
fi

# ---------------------------------------------------------------------------
# Collect and summarize results
# ---------------------------------------------------------------------------
echo "=== Results Summary ==="
echo ""

SUMMARY_FILE="$RESULTS_DIR/summary-$TIMESTAMP.json"

# Merge all summary files into one
python3 - "$RESULTS_DIR" "$TIMESTAMP" "$SUMMARY_FILE" <<'PYEOF'
import json, sys, os, glob

results_dir = sys.argv[1]
ts = sys.argv[2]
output = sys.argv[3]

summary = {"timestamp": ts, "scenarios": {}}

for name in ["ingest", "delivery", "pull", "d1"]:
    path = os.path.join(results_dir, f"{name}-summary-{ts}.json")
    if os.path.exists(path):
        with open(path) as f:
            data = json.load(f)
        metrics = data.get("metrics", {})
        scenario = {}
        for key in ["http_req_duration", "http_reqs", "iterations"]:
            if key in metrics:
                scenario[key] = metrics[key]
        # Custom metrics
        for key in metrics:
            if key.startswith("queue_") or key.startswith("event_") or key.startswith("delivery_"):
                scenario[key] = metrics[key]
        summary["scenarios"][name] = scenario

with open(output, "w") as f:
    json.dump(summary, f, indent=2)

print(json.dumps(summary, indent=2))
PYEOF

echo ""
echo "Full results saved to: $RESULTS_DIR/"
echo "  metadata:  metadata-$TIMESTAMP.json"
echo "  summary:   summary-$TIMESTAMP.json"
echo ""
echo "=== Done ==="
