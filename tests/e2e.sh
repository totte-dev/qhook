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
  allow_private_urls: true
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
  allow_private_urls: true
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
  allow_private_urls: true
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
  allow_private_urls: true
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
echo "=== Test 6: CloudEvents binary mode ==="
########################################

start_mock 19005

cat > /tmp/e2e_qhook_6.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_6.db?mode=rwc"
server:
  port: 19106
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  on-ce:
    source: app
    events: [com.example.order.created]
    url: http://127.0.0.1:19005/jobs/ce
EOF

start_qhook /tmp/e2e_qhook_6.yaml

# CloudEvents binary mode: event type from ce-type header (overrides URL path)
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19106/events/ignored.by.header \
    -H "Content-Type: application/json" \
    -H "ce-type: com.example.order.created" \
    -H "ce-source: /myapp" \
    -H "ce-id: evt-ce-001" \
    -H "ce-specversion: 1.0" \
    -d '{"orderId": "ord_ce_1"}')

[ "$HTTP_CODE" = "202" ] && pass "CloudEvents binary mode accepted (202)" || fail "CE binary" "got $HTTP_CODE"

sleep 2

COUNT=$(curl -s --max-time 3 http://127.0.0.1:19005/count 2>/dev/null || echo "0")
[ "$COUNT" -ge 1 ] 2>/dev/null && pass "CloudEvents event matched handler (count=$COUNT)" || fail "CE match" "count=$COUNT"

# Check that ce-* headers are forwarded
RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19005/received 2>/dev/null || echo "[]")
HAS_CE=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
if d:
    h = d[0].get('headers', {})
    has_type = any('ce-type' in k.lower() for k in h)
    has_source = any('ce-source' in k.lower() for k in h)
    print('yes' if has_type and has_source else 'no')
else:
    print('no')
" 2>/dev/null || echo "no")
[ "$HAS_CE" = "yes" ] && pass "CloudEvents headers forwarded" || fail "CE headers" "ce-type/ce-source not found"

########################################
echo ""
echo "=== Test 7: Event filtering ==="
########################################

start_mock 19006

cat > /tmp/e2e_qhook_7.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_7.db?mode=rwc"
server:
  port: 19107
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  paid-only:
    source: app
    events: [order.*]
    url: http://127.0.0.1:19006/jobs/paid
    filter: "$.status == paid"
EOF

start_qhook /tmp/e2e_qhook_7.yaml

# Should be filtered OUT (status != paid)
curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19107/events/order.created \
    -H "Content-Type: application/json" \
    -d '{"status": "pending", "id": "ord_1"}'

# Should pass filter (status == paid)
curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19107/events/order.updated \
    -H "Content-Type: application/json" \
    -d '{"status": "paid", "id": "ord_2"}'

sleep 2

COUNT=$(curl -s --max-time 3 http://127.0.0.1:19006/count 2>/dev/null || echo "0")
[ "$COUNT" = "1" ] && pass "Filter: only paid event delivered (count=$COUNT)" || fail "Filter" "expected 1, got $COUNT"

########################################
echo ""
echo "=== Test 8: Payload transformation ==="
########################################

start_mock 19007

cat > /tmp/e2e_qhook_8.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_8.db?mode=rwc"
server:
  port: 19108
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  transform-test:
    source: app
    events: [transform.test]
    url: http://127.0.0.1:19007/jobs/transform
    transform: '{"event_id": "{{$.id}}", "amount": {{$.data.amount}}}'
EOF

start_qhook /tmp/e2e_qhook_8.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19108/events/transform.test \
    -H "Content-Type: application/json" \
    -d '{"id": "evt_t1", "data": {"amount": 42, "extra": "ignored"}}'

sleep 2

RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19007/received 2>/dev/null || echo "[]")
TRANSFORMED=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
if d:
    body = json.loads(d[0].get('body', '{}'))
    has_id = body.get('event_id') == 'evt_t1'
    has_amount = body.get('amount') == 42
    no_extra = 'extra' not in body
    print('yes' if has_id and has_amount and no_extra else 'no')
else:
    print('no')
" 2>/dev/null || echo "no")
[ "$TRANSFORMED" = "yes" ] && pass "Transform: payload reshaped correctly" || fail "Transform" "unexpected payload"

########################################
echo ""
echo "=== Test 9: IP rate limiting ==="
########################################

start_mock 19008

cat > /tmp/e2e_qhook_9.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_9.db?mode=rwc"
server:
  port: 19109
  ip_rate_limit: 3
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  rate-test:
    source: app
    events: [rate.test]
    url: http://127.0.0.1:19008/jobs/rate
EOF

start_qhook /tmp/e2e_qhook_9.yaml

# Send 5 requests rapidly — first 3 should succeed, rest should get 429
CODES=""
for i in $(seq 1 5); do
    CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
        -X POST http://127.0.0.1:19109/events/rate.test \
        -H "Content-Type: application/json" \
        -d "{\"i\": $i}")
    CODES="$CODES $CODE"
done

HAS_429=$(echo "$CODES" | grep -c "429" || true)
[ "$HAS_429" -ge 1 ] && pass "IP rate limit: 429 returned ($CODES)" || fail "IP rate limit" "no 429 in:$CODES"

########################################
echo ""
echo "=== Test 10: auth_token protection ==="
########################################

cat > /tmp/e2e_qhook_10.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_10.db?mode=rwc"
server:
  port: 19110
  allow_private_urls: true
api:
  auth_token: secret-token-123
sources:
  app:
    type: event
handlers:
  auth-test:
    source: app
    events: [auth.test]
    url: http://127.0.0.1:19008/jobs/auth
EOF

start_qhook /tmp/e2e_qhook_10.yaml

# Without token → 401
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19110/events/auth.test \
    -H "Content-Type: application/json" \
    -d '{"test": true}')
[ "$HTTP_CODE" = "401" ] && pass "Missing auth token rejected (401)" || fail "Auth missing" "got $HTTP_CODE"

# With wrong token → 401
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19110/events/auth.test \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer wrong-token" \
    -d '{"test": true}')
[ "$HTTP_CODE" = "401" ] && pass "Wrong auth token rejected (401)" || fail "Auth wrong" "got $HTTP_CODE"

# With correct token → 202
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19110/events/auth.test \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer secret-token-123" \
    -d '{"test": true}')
[ "$HTTP_CODE" = "202" ] && pass "Correct auth token accepted (202)" || fail "Auth correct" "got $HTTP_CODE"

########################################
echo ""
echo "=== Test 11: HTTP method specification ==="
########################################

start_mock 19111
cat > /tmp/e2e_qhook_11.yaml <<EOF
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_11.db?mode=rwc"
server:
  port: 19211
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  put-handler:
    source: app
    events: ["resource.update"]
    url: http://127.0.0.1:19111/jobs/put
    method: PUT
    retry: { max: 0 }
  get-handler:
    source: app
    events: ["resource.check"]
    url: http://127.0.0.1:19111/jobs/get
    method: GET
    retry: { max: 0 }
  default-handler:
    source: app
    events: ["resource.create"]
    url: http://127.0.0.1:19111/jobs/post
    retry: { max: 0 }
EOF
start_qhook /tmp/e2e_qhook_11.yaml

# Send events
curl -sf -X POST http://127.0.0.1:19211/events/resource.update \
    -H "Content-Type: application/json" \
    -d '{"id": "res_1"}' > /dev/null

curl -sf -X POST http://127.0.0.1:19211/events/resource.check \
    -H "Content-Type: application/json" \
    -d '{"id": "res_2"}' > /dev/null

curl -sf -X POST http://127.0.0.1:19211/events/resource.create \
    -H "Content-Type: application/json" \
    -d '{"id": "res_3"}' > /dev/null

sleep 3

# Check delivery methods
RECEIVED=$(curl -sf http://127.0.0.1:19111/received 2>/dev/null || echo "[]")
PUT_METHOD=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/put' in r.get('path', ''):
        print(r.get('method', 'unknown'))
        break
else:
    print('none')
" 2>/dev/null || echo "none")
[ "$PUT_METHOD" = "PUT" ] && pass "PUT method used for put-handler" || fail "PUT method" "got $PUT_METHOD"

GET_METHOD=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/get' in r.get('path', ''):
        print(r.get('method', 'unknown'))
        break
else:
    print('none')
" 2>/dev/null || echo "none")
[ "$GET_METHOD" = "GET" ] && pass "GET method used for get-handler" || fail "GET method" "got $GET_METHOD"

POST_METHOD=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/post' in r.get('path', ''):
        print(r.get('method', 'unknown'))
        break
else:
    print('none')
" 2>/dev/null || echo "none")
[ "$POST_METHOD" = "POST" ] && pass "POST method used by default" || fail "POST default" "got $POST_METHOD"

# Check GET has no body
GET_BODY=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/get' in r.get('path', ''):
        print('empty' if not r.get('body', '') else 'has_body')
        break
else:
    print('none')
" 2>/dev/null || echo "none")
[ "$GET_BODY" = "empty" ] && pass "GET request has no body" || fail "GET body" "got $GET_BODY"

########################################
echo ""
echo "=== Test 12: Cron trigger ==="
########################################

start_mock 19112
cat > /tmp/e2e_qhook_12.yaml <<EOF
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_qhook_12.db?mode=rwc"
server:
  port: 19212
  allow_private_urls: true
sources:
  heartbeat:
    type: cron
    schedule: "*/3 * * * * *"
handlers:
  on-tick:
    source: heartbeat
    url: http://127.0.0.1:19112/jobs/cron
    retry: { max: 0 }
EOF
start_qhook /tmp/e2e_qhook_12.yaml

# Wait for at least one cron fire (3 seconds + margin)
sleep 5

COUNT=$(curl -sf http://127.0.0.1:19112/count 2>/dev/null || echo "0")
[ "$COUNT" -ge 1 ] 2>/dev/null && pass "Cron trigger fired (count=$COUNT)" || fail "Cron trigger" "count=$COUNT"

# Check payload contains source and fired_at
RECEIVED=$(curl -sf http://127.0.0.1:19112/received 2>/dev/null || echo "[]")
HAS_SOURCE=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
if d:
    body = d[0].get('body', '')
    print('yes' if 'heartbeat' in body and 'fired_at' in body else 'no')
else:
    print('no')
" 2>/dev/null || echo "no")
[ "$HAS_SOURCE" = "yes" ] && pass "Cron payload has source and fired_at" || fail "Cron payload" "missing fields"

########################################
echo ""
echo "==============================="
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC}"
echo "==============================="

[ $FAIL -eq 0 ] && exit 0 || exit 1
