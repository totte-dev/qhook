#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

PASS=0
FAIL=0
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

pass() { echo -e "  ${GREEN}PASS${NC}: $1"; PASS=$((PASS+1)); }
fail() { echo -e "  ${RED}FAIL${NC}: $1 — $2"; FAIL=$((FAIL+1)); }

PIDS=()
cleanup() {
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    rm -f /tmp/e2e_qhook_*.yaml /tmp/e2e_qhook_*.db
}
trap cleanup EXIT

start_mock() {
    local port=$1
    local fail_n=${2:-0}
    python3 "$SCRIPT_DIR/mock_server.py" "$port" "$fail_n" &
    PIDS+=($!)
    sleep 0.3
}

start_qhook() {
    local config=$1
    ./target/debug/qhook start --config "$config" 2>/dev/null &
    PIDS+=($!)
    sleep 1
}

echo "Building qhook..."
cargo build --quiet 2>&1 | grep -v warning || true

########################################
echo ""
echo "=== Test 1: Internal event → delivery ==="
########################################

start_mock 19001

cat > /tmp/e2e_qhook_1.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_1.db?mode=rwc"
server:
  port: 19101
sources:
  app:
    type: event
handlers:
  on-test:
    source: app
    events: [test.hello]
    url: http://127.0.0.1:19001/jobs/test
    retry: { max: 3 }
EOF

start_qhook /tmp/e2e_qhook_1.yaml

# Send event
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19101/events/test.hello \
    -H "Content-Type: application/json" \
    -d '{"message": "hello"}')

[ "$HTTP_CODE" = "202" ] && pass "Event accepted (202)" || fail "Event accepted" "got $HTTP_CODE"

sleep 2

COUNT=$(curl -s --max-time 3 http://127.0.0.1:19001/count 2>/dev/null || echo "0")
[ "$COUNT" -ge 1 ] 2>/dev/null && pass "Delivery received (count=$COUNT)" || fail "Delivery" "count=$COUNT"

# Check payload + headers
RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19001/received 2>/dev/null || echo "[]")
HAS_PAYLOAD=$(echo "$RECEIVED" | python3 -c "import sys,json;d=json.load(sys.stdin);print('yes' if d and 'hello' in d[0].get('body','') else 'no')" 2>/dev/null || echo "no")
[ "$HAS_PAYLOAD" = "yes" ] && pass "Payload preserved" || fail "Payload" "not found"

HAS_HEADERS=$(echo "$RECEIVED" | python3 -c "import sys,json;d=json.load(sys.stdin);h=d[0].get('headers',{}) if d else {};print('yes' if any('qhook' in k.lower() for k in h) else 'no')" 2>/dev/null || echo "no")
[ "$HAS_HEADERS" = "yes" ] && pass "X-Qhook headers present" || fail "Headers" "not found"

########################################
echo ""
echo "=== Test 2: Unknown webhook → 404 ==="
########################################

HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19101/webhooks/nonexistent \
    -d '{}')

[ "$HTTP_CODE" = "404" ] && pass "Unknown source returns 404" || fail "Unknown source" "got $HTTP_CODE"

########################################
echo ""
echo "=== Test 3: Idempotency ==="
########################################

start_mock 19002

cat > /tmp/e2e_qhook_3.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_3.db?mode=rwc"
server:
  port: 19103
sources:
  app:
    type: event
handlers:
  dedup:
    source: app
    events: [dedup.test]
    url: http://127.0.0.1:19002/jobs/dedup
    idempotency_key: "$.id"
EOF

start_qhook /tmp/e2e_qhook_3.yaml

# Send same event twice
curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19103/events/dedup.test \
    -H "Content-Type: application/json" \
    -d '{"id": "evt_123", "data": "first"}'

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19103/events/dedup.test \
    -H "Content-Type: application/json" \
    -d '{"id": "evt_123", "data": "duplicate"}'

sleep 2

COUNT=$(curl -s --max-time 3 http://127.0.0.1:19002/count 2>/dev/null || echo "?")
[ "$COUNT" = "1" ] && pass "Duplicate deduplicated (count=$COUNT)" || fail "Dedup" "expected 1, got $COUNT"

########################################
echo ""
echo "=== Test 4: GitHub signature verification ==="
########################################

start_mock 19003

cat > /tmp/e2e_qhook_4.yaml <<EOF
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_4.db?mode=rwc"
server:
  port: 19104
sources:
  github:
    type: webhook
    verify: github
    secret: test-secret-123
handlers:
  on-push:
    source: github
    events: [push]
    url: http://127.0.0.1:19003/jobs/deploy
EOF

start_qhook /tmp/e2e_qhook_4.yaml

PAYLOAD='{"action":"push","ref":"refs/heads/main"}'
SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "test-secret-123" | awk '{print $NF}')

# Valid signature
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19104/webhooks/github \
    -H "Content-Type: application/json" \
    -H "X-Hub-Signature-256: sha256=$SIGNATURE" \
    -d "$PAYLOAD")
[ "$HTTP_CODE" = "200" ] && pass "Valid signature accepted (200)" || fail "Valid sig" "got $HTTP_CODE"

# Invalid signature
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19104/webhooks/github \
    -H "Content-Type: application/json" \
    -H "X-Hub-Signature-256: sha256=invalid" \
    -d "$PAYLOAD")
[ "$HTTP_CODE" = "401" ] && pass "Invalid signature rejected (401)" || fail "Invalid sig" "got $HTTP_CODE"

# Missing signature
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19104/webhooks/github \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD")
[ "$HTTP_CODE" = "401" ] && pass "Missing signature rejected (401)" || fail "Missing sig" "got $HTTP_CODE"

sleep 2
COUNT=$(curl -s --max-time 3 http://127.0.0.1:19003/count 2>/dev/null || echo "0")
[ "$COUNT" = "1" ] && pass "Verified webhook delivered (count=$COUNT)" || fail "Webhook delivery" "got $COUNT"

########################################
echo ""
echo "=== Test 5: Retry on failure ==="
########################################

start_mock 19004 2  # Fail first 2 requests

cat > /tmp/e2e_qhook_5.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_5.db?mode=rwc"
server:
  port: 19105
sources:
  app:
    type: event
handlers:
  retry-test:
    source: app
    events: [retry.test]
    url: http://127.0.0.1:19004/jobs/retry
    retry: { max: 5 }
EOF

start_qhook /tmp/e2e_qhook_5.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19105/events/retry.test \
    -H "Content-Type: application/json" \
    -d '{"retry": true}'

sleep 2

COUNT=$(curl -s --max-time 3 http://127.0.0.1:19004/count 2>/dev/null || echo "0")
[ "$COUNT" -ge 1 ] 2>/dev/null && pass "First attempt made (count=$COUNT, expected failure)" || fail "Retry" "no attempt"

########################################
echo ""
echo "==============================="
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC}"
echo "==============================="

[ $FAIL -eq 0 ] && exit 0 || exit 1
