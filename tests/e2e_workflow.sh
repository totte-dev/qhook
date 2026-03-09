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
    rm -f /tmp/e2e_wf_*.yaml /tmp/e2e_wf_*.db
}
trap cleanup EXIT

start_mock() {
    local port=$1
    shift
    python3 "$SCRIPT_DIR/mock_workflow_server.py" "$port" "$@" &
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
echo "=== Workflow Test 1: 3-step sequential pipeline ==="
########################################

start_mock 19201

cat > /tmp/e2e_wf_1.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_1.db?mode=rwc"
server:
  port: 19301
sources:
  app:
    type: event
handlers: {}
workflows:
  order-flow:
    source: app
    events: [order.created]
    steps:
      - name: validate
        url: http://127.0.0.1:19201/validate
      - name: fulfill
        url: http://127.0.0.1:19201/fulfill
      - name: notify
        url: http://127.0.0.1:19201/notify
EOF

start_qhook /tmp/e2e_wf_1.yaml

# Send event to trigger workflow
HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST http://127.0.0.1:19301/events/order.created \
    -H "Content-Type: application/json" \
    -d '{"id": "ord_001", "amount": 5000}')

[ "$HTTP_CODE" = "202" ] && pass "Workflow event accepted (202)" || fail "Event accept" "got $HTTP_CODE"

# Wait for all 3 steps to complete
sleep 5

# Check that all 3 endpoints were called
TOTAL=$(curl -s --max-time 3 http://127.0.0.1:19201/count 2>/dev/null || echo "0")
[ "$TOTAL" -ge 3 ] 2>/dev/null && pass "All 3 steps executed (count=$TOTAL)" || fail "Step count" "expected >=3, got $TOTAL"

VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19201/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" -ge 1 ] 2>/dev/null && pass "Step 1 (validate) executed" || fail "Step 1" "count=$VALIDATE_COUNT"

FULFILL_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19201/count/fulfill 2>/dev/null || echo "0")
[ "$FULFILL_COUNT" -ge 1 ] 2>/dev/null && pass "Step 2 (fulfill) executed" || fail "Step 2" "count=$FULFILL_COUNT"

NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19201/count/notify 2>/dev/null || echo "0")
[ "$NOTIFY_COUNT" -ge 1 ] 2>/dev/null && pass "Step 3 (notify) executed" || fail "Step 3" "count=$NOTIFY_COUNT"

# Check that step 2 received step 1's response (response chaining)
RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19201/received 2>/dev/null || echo "[]")
STEP2_INPUT=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
# Find the fulfill request (step 2)
for r in d:
    if '/fulfill' in r['path']:
        body = json.loads(r['body'])
        # Step 2 should receive step 1's response (which has 'valid' field)
        print('yes' if 'valid' in body else 'no')
        sys.exit()
print('no')
" 2>/dev/null || echo "no")
[ "$STEP2_INPUT" = "yes" ] && pass "Step 2 received step 1 response (data chaining)" || fail "Data chaining" "step 2 didn't get step 1 response"

# Check workflow-runs CLI
WF_RUNS=$(./target/debug/qhook workflow-runs list --config /tmp/e2e_wf_1.yaml 2>/dev/null)
HAS_COMPLETED=$(echo "$WF_RUNS" | grep -c "completed" || true)
[ "$HAS_COMPLETED" -ge 1 ] && pass "workflow-runs shows completed" || fail "CLI" "no completed run: $WF_RUNS"

########################################
echo ""
echo "=== Workflow Test 2: input transform + result_path ==="
########################################

start_mock 19202

cat > /tmp/e2e_wf_2.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_2.db?mode=rwc"
server:
  port: 19302
sources:
  app:
    type: event
handlers: {}
workflows:
  enrich-flow:
    source: app
    events: [user.signup]
    steps:
      - name: enrich
        url: http://127.0.0.1:19202/validate
        input: '{"user_id": "{{$.id}}"}'
        result_path: "$.enrichment"
      - name: complete
        url: http://127.0.0.1:19202/fulfill
EOF

start_qhook /tmp/e2e_wf_2.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19302/events/user.signup \
    -H "Content-Type: application/json" \
    -d '{"id": "usr_001", "name": "Alice"}'

sleep 4

# Step 1 should receive transformed input
RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19202/received 2>/dev/null || echo "[]")
STEP1_TRANSFORMED=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/validate' in r['path']:
        body = json.loads(r['body'])
        # Should be the transformed input: {user_id: 'usr_001'}
        print('yes' if body.get('user_id') == 'usr_001' and 'name' not in body else 'no')
        sys.exit()
print('no')
" 2>/dev/null || echo "no")
[ "$STEP1_TRANSFORMED" = "yes" ] && pass "input transform applied" || fail "Input transform" "step 1 didn't get transformed input"

# Step 2 should receive merged result (original input + step1 response under $.enrichment)
STEP2_MERGED=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/fulfill' in r['path']:
        body = json.loads(r['body'])
        # result_path=$.enrichment means step1 response merged under 'enrichment' key
        # The original input was {user_id: 'usr_001'} (after transform)
        has_enrichment = 'enrichment' in body
        enrichment_has_valid = body.get('enrichment', {}).get('valid') == True
        print('yes' if has_enrichment and enrichment_has_valid else 'no')
        sys.exit()
print('no')
" 2>/dev/null || echo "no")
[ "$STEP2_MERGED" = "yes" ] && pass "result_path merge: response under $.enrichment" || fail "result_path" "merge not correct"

########################################
echo ""
echo "=== Workflow Test 3: on_failure=continue ==="
########################################

# Mock that fails /validate with 500
start_mock 19203 "/validate:500:99"

cat > /tmp/e2e_wf_3.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_3.db?mode=rwc"
server:
  port: 19303
delivery:
  default_retry:
    max: 1
sources:
  app:
    type: event
handlers: {}
workflows:
  continue-flow:
    source: app
    events: [test.continue]
    steps:
      - name: might-fail
        url: http://127.0.0.1:19203/validate
        on_failure: continue
        retry: { max: 1 }
      - name: always-runs
        url: http://127.0.0.1:19203/notify
EOF

start_qhook /tmp/e2e_wf_3.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19303/events/test.continue \
    -H "Content-Type: application/json" \
    -d '{"test": true}'

sleep 4

# Step 1 fails, but step 2 should still run due to on_failure=continue
NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19203/count/notify 2>/dev/null || echo "0")
[ "$NOTIFY_COUNT" -ge 1 ] 2>/dev/null && pass "on_failure=continue: step 2 ran after step 1 failure" || fail "on_failure=continue" "notify count=$NOTIFY_COUNT"

# Step 2 should receive error info from step 1
RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19203/received 2>/dev/null || echo "[]")
HAS_ERROR_INFO=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/notify' in r['path']:
        body = json.loads(r['body'])
        print('yes' if 'error' in body and 'failed_step' in body else 'no')
        sys.exit()
print('no')
" 2>/dev/null || echo "no")
[ "$HAS_ERROR_INFO" = "yes" ] && pass "on_failure=continue: error info passed to next step" || fail "Error info" "not found"

########################################
echo ""
echo "=== Workflow Test 4: catch error routing ==="
########################################

# Mock that fails /validate with 400 (4xx)
start_mock 19204 "/validate:400:99"

cat > /tmp/e2e_wf_4.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_4.db?mode=rwc"
server:
  port: 19304
sources:
  app:
    type: event
handlers: {}
workflows:
  catch-flow:
    source: app
    events: [test.catch]
    steps:
      - name: validate
        url: http://127.0.0.1:19204/validate
        retry:
          max: 1
          errors: [5xx, timeout]
        catch:
          - errors: [4xx]
            goto: handle-error
          - errors: [all]
            goto: alert
      - name: should-not-run
        url: http://127.0.0.1:19204/fulfill
      - name: handle-error
        url: http://127.0.0.1:19204/bad-request
        end: true
      - name: alert
        url: http://127.0.0.1:19204/alert
        end: true
EOF

start_qhook /tmp/e2e_wf_4.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19304/events/test.catch \
    -H "Content-Type: application/json" \
    -d '{"test": true}'

sleep 4

# validate returns 400 → catch routes to handle-error (not fulfill, not alert)
BAD_REQ_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19204/count/bad-request 2>/dev/null || echo "0")
[ "$BAD_REQ_COUNT" -ge 1 ] 2>/dev/null && pass "catch: 4xx routed to handle-error" || fail "Catch 4xx" "bad-request count=$BAD_REQ_COUNT"

FULFILL_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19204/count/fulfill 2>/dev/null || echo "0")
[ "$FULFILL_COUNT" = "0" ] && pass "catch: should-not-run was skipped" || fail "Catch skip" "fulfill count=$FULFILL_COUNT"

ALERT_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19204/count/alert 2>/dev/null || echo "0")
[ "$ALERT_COUNT" = "0" ] && pass "catch: alert step was not triggered (4xx matched first)" || fail "Catch order" "alert count=$ALERT_COUNT"

########################################
echo ""
echo "=== Workflow Test 5: end step terminates workflow ==="
########################################

start_mock 19205

cat > /tmp/e2e_wf_5.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_5.db?mode=rwc"
server:
  port: 19305
sources:
  app:
    type: event
handlers: {}
workflows:
  end-flow:
    source: app
    events: [test.end]
    steps:
      - name: first
        url: http://127.0.0.1:19205/validate
        end: true
      - name: should-not-run
        url: http://127.0.0.1:19205/fulfill
EOF

start_qhook /tmp/e2e_wf_5.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19305/events/test.end \
    -H "Content-Type: application/json" \
    -d '{"test": true}'

sleep 3

VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19205/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" -ge 1 ] 2>/dev/null && pass "end: first step executed" || fail "End step" "validate count=$VALIDATE_COUNT"

FULFILL_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19205/count/fulfill 2>/dev/null || echo "0")
[ "$FULFILL_COUNT" = "0" ] && pass "end: second step skipped" || fail "End skip" "fulfill count=$FULFILL_COUNT"

########################################
echo ""
echo "=== Workflow Test 6: handlers + workflows coexist ==="
########################################

start_mock 19206

cat > /tmp/e2e_wf_6.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_6.db?mode=rwc"
server:
  port: 19306
sources:
  app:
    type: event
handlers:
  simple:
    source: app
    events: [dual.test]
    url: http://127.0.0.1:19206/notify
workflows:
  dual-flow:
    source: app
    events: [dual.test]
    steps:
      - name: process
        url: http://127.0.0.1:19206/validate
EOF

start_qhook /tmp/e2e_wf_6.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19306/events/dual.test \
    -H "Content-Type: application/json" \
    -d '{"dual": true}'

sleep 3

# Both handler and workflow should fire
NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19206/count/notify 2>/dev/null || echo "0")
[ "$NOTIFY_COUNT" -ge 1 ] 2>/dev/null && pass "coexist: handler fired" || fail "Handler" "count=$NOTIFY_COUNT"

VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19206/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" -ge 1 ] 2>/dev/null && pass "coexist: workflow fired" || fail "Workflow" "count=$VALIDATE_COUNT"

########################################
echo ""
echo "=== Workflow Test 7: choice step routing ==="
########################################

start_mock 19207

cat > /tmp/e2e_wf_7.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_7.db?mode=rwc"
server:
  port: 19307
sources:
  app:
    type: event
handlers: {}
workflows:
  routing-flow:
    source: app
    events: [order.route]
    steps:
      - name: route
        type: choice
        choices:
          - when: "$.amount >= 10000"
            goto: high-value
          - when: "$.category == premium"
            goto: premium
        default: standard
      - name: high-value
        url: http://127.0.0.1:19207/fulfill
        end: true
      - name: premium
        url: http://127.0.0.1:19207/notify
        end: true
      - name: standard
        url: http://127.0.0.1:19207/validate
        end: true
EOF

start_qhook /tmp/e2e_wf_7.yaml

# Test: high value order (amount >= 10000)
curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19307/events/order.route \
    -H "Content-Type: application/json" \
    -d '{"amount": 15000, "category": "regular"}'

sleep 3

FULFILL_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19207/count/fulfill 2>/dev/null || echo "0")
[ "$FULFILL_COUNT" -ge 1 ] 2>/dev/null && pass "choice: high-value route (amount>=10000)" || fail "Choice high" "fulfill=$FULFILL_COUNT"

# Test: premium category (second rule)
curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19307/events/order.route \
    -H "Content-Type: application/json" \
    -d '{"amount": 500, "category": "premium"}'

sleep 3

NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19207/count/notify 2>/dev/null || echo "0")
[ "$NOTIFY_COUNT" -ge 1 ] 2>/dev/null && pass "choice: premium route (category)" || fail "Choice premium" "notify=$NOTIFY_COUNT"

# Test: default route (no match)
curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19307/events/order.route \
    -H "Content-Type: application/json" \
    -d '{"amount": 500, "category": "regular"}'

sleep 3

VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19207/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" -ge 1 ] 2>/dev/null && pass "choice: default route" || fail "Choice default" "validate=$VALIDATE_COUNT"

########################################
echo ""
echo "=== Workflow Test 8: parallel step execution ==="
########################################

start_mock 19208

cat > /tmp/e2e_wf_8.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_8.db?mode=rwc"
server:
  port: 19308
sources:
  app:
    type: event
handlers: {}
workflows:
  parallel-flow:
    source: app
    events: [check.all]
    steps:
      - name: checks
        type: parallel
        branches:
          - name: credit
            url: http://127.0.0.1:19208/validate
          - name: fraud
            url: http://127.0.0.1:19208/fulfill
        result_path: "$.checks"
      - name: finalize
        url: http://127.0.0.1:19208/notify
EOF

start_qhook /tmp/e2e_wf_8.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19308/events/check.all \
    -H "Content-Type: application/json" \
    -d '{"user_id": "usr_001"}'

sleep 5

# Both branches should execute
VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19208/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" -ge 1 ] 2>/dev/null && pass "parallel: credit branch executed" || fail "Parallel credit" "count=$VALIDATE_COUNT"

FULFILL_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19208/count/fulfill 2>/dev/null || echo "0")
[ "$FULFILL_COUNT" -ge 1 ] 2>/dev/null && pass "parallel: fraud branch executed" || fail "Parallel fraud" "count=$FULFILL_COUNT"

# Finalize step should have received merged results
NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19208/count/notify 2>/dev/null || echo "0")
[ "$NOTIFY_COUNT" -ge 1 ] 2>/dev/null && pass "parallel: finalize step ran after branches" || fail "Parallel finalize" "count=$NOTIFY_COUNT"

# Verify finalize received merged parallel results under $.checks
RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19208/received 2>/dev/null || echo "[]")
HAS_CHECKS=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/notify' in r['path']:
        body = json.loads(r['body'])
        has_checks = 'checks' in body
        has_credit = 'credit' in body.get('checks', {})
        has_fraud = 'fraud' in body.get('checks', {})
        print('yes' if has_checks and has_credit and has_fraud else 'no')
        sys.exit()
print('no')
" 2>/dev/null || echo "no")
[ "$HAS_CHECKS" = "yes" ] && pass "parallel: results merged under $.checks" || fail "Parallel merge" "checks not found"

########################################
echo ""
echo "=== Workflow Test 9: map step execution ==="
########################################

start_mock 19209

cat > /tmp/e2e_wf_9.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_9.db?mode=rwc"
server:
  port: 19309
sources:
  app:
    type: event
handlers: {}
workflows:
  map-flow:
    source: app
    events: [batch.process]
    steps:
      - name: process-items
        type: map
        items_path: "$.items"
        url: http://127.0.0.1:19209/validate
        result_path: "$.results"
      - name: summarize
        url: http://127.0.0.1:19209/notify
EOF

start_qhook /tmp/e2e_wf_9.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19309/events/batch.process \
    -H "Content-Type: application/json" \
    -d '{"items": [{"id": 1}, {"id": 2}, {"id": 3}]}'

sleep 5

# All 3 items should be processed
VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19209/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" -ge 3 ] 2>/dev/null && pass "map: all 3 items processed (count=$VALIDATE_COUNT)" || fail "Map items" "count=$VALIDATE_COUNT"

# Summarize step should run after all items
NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19209/count/notify 2>/dev/null || echo "0")
[ "$NOTIFY_COUNT" -ge 1 ] 2>/dev/null && pass "map: summarize step ran after items" || fail "Map summarize" "count=$NOTIFY_COUNT"

# Verify summarize received array of results under $.results
RECEIVED=$(curl -s --max-time 3 http://127.0.0.1:19209/received 2>/dev/null || echo "[]")
HAS_RESULTS=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d:
    if '/notify' in r['path']:
        body = json.loads(r['body'])
        results = body.get('results', [])
        print('yes' if isinstance(results, list) and len(results) == 3 else 'no')
        sys.exit()
print('no')
" 2>/dev/null || echo "no")
[ "$HAS_RESULTS" = "yes" ] && pass "map: results merged as array (3 items)" || fail "Map merge" "results not correct"

########################################
echo ""
echo "=== Workflow Test 10: wait step (fixed seconds) ==="
########################################

start_mock 19210

cat > /tmp/e2e_wf_10.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_10.db?mode=rwc"
server:
  port: 19310
sources:
  app:
    type: event
handlers: {}
workflows:
  delayed-flow:
    source: app
    events: [test.wait]
    steps:
      - name: delay
        type: wait
        seconds: 2
      - name: process
        url: http://127.0.0.1:19210/validate
EOF

start_qhook /tmp/e2e_wf_10.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19310/events/test.wait \
    -H "Content-Type: application/json" \
    -d '{"test": "wait"}'

# Check that process has NOT run yet (wait is 2 seconds)
sleep 1
EARLY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19210/count/validate 2>/dev/null || echo "0")
[ "$EARLY_COUNT" = "0" ] && pass "wait: step not executed before delay" || fail "Wait early" "count=$EARLY_COUNT"

# Wait for the delay + processing
sleep 5

VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19210/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" -ge 1 ] 2>/dev/null && pass "wait: step executed after delay" || fail "Wait delayed" "count=$VALIDATE_COUNT"

########################################
echo ""
echo "=== Workflow Test 11: callback step ==="
########################################

start_mock 19211

cat > /tmp/e2e_wf_11.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_11.db?mode=rwc"
server:
  port: 19311
sources:
  app:
    type: event
handlers: {}
workflows:
  approval-flow:
    source: app
    events: [test.callback]
    steps:
      - name: request
        url: http://127.0.0.1:19211/validate
      - name: wait-approval
        type: callback
      - name: finalize
        url: http://127.0.0.1:19211/notify
EOF

start_qhook /tmp/e2e_wf_11.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19311/events/test.callback \
    -H "Content-Type: application/json" \
    -d '{"order": "ord_001"}'

# Wait for step 1 to complete and callback to be created
sleep 4

# Step 1 should have executed
REQUEST_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19211/count/validate 2>/dev/null || echo "0")
[ "$REQUEST_COUNT" -ge 1 ] 2>/dev/null && pass "callback: step 1 executed" || fail "Callback step 1" "count=$REQUEST_COUNT"

# Finalize should NOT have run yet (waiting for callback)
NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19211/count/notify 2>/dev/null || echo "0")
[ "$NOTIFY_COUNT" = "0" ] && pass "callback: finalize not run yet (waiting)" || fail "Callback waiting" "count=$NOTIFY_COUNT"

# Find the callback token from the DB
CALLBACK_TOKEN=$(sqlite3 /tmp/e2e_wf_11.db "SELECT callback_token FROM jobs WHERE callback_token IS NOT NULL LIMIT 1" 2>/dev/null || echo "")
[ -n "$CALLBACK_TOKEN" ] && pass "callback: token created in DB" || fail "Callback token" "not found"

# Send the callback
if [ -n "$CALLBACK_TOKEN" ]; then
    CB_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
        -X POST "http://127.0.0.1:19311/callback/$CALLBACK_TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"approved": true}')
    [ "$CB_CODE" = "200" ] && pass "callback: API returned 200" || fail "Callback API" "got $CB_CODE"

    sleep 3

    # Finalize should now have run
    NOTIFY_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19211/count/notify 2>/dev/null || echo "0")
    [ "$NOTIFY_COUNT" -ge 1 ] 2>/dev/null && pass "callback: finalize ran after callback" || fail "Callback finalize" "count=$NOTIFY_COUNT"
fi

########################################
echo ""
echo "=== Workflow Test 12: workflow timeout ==="
########################################

start_mock 19212

cat > /tmp/e2e_wf_12.yaml <<'EOF'
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_wf_12.db?mode=rwc"
server:
  port: 19312
sources:
  app:
    type: event
handlers: {}
workflows:
  timeout-flow:
    source: app
    events: [test.timeout]
    timeout: 3
    steps:
      - name: delay
        type: wait
        seconds: 5
      - name: should-not-run
        url: http://127.0.0.1:19212/validate
EOF

start_qhook /tmp/e2e_wf_12.yaml

curl -s --max-time 3 -o /dev/null \
    -X POST http://127.0.0.1:19312/events/test.timeout \
    -H "Content-Type: application/json" \
    -d '{"test": "timeout"}'

# Wait for the workflow to try processing after wait
sleep 8

# The second step should NOT run because the workflow times out
VALIDATE_COUNT=$(curl -s --max-time 3 http://127.0.0.1:19212/count/validate 2>/dev/null || echo "0")
[ "$VALIDATE_COUNT" = "0" ] && pass "timeout: step not executed (workflow timed out)" || fail "Timeout" "count=$VALIDATE_COUNT"

# Check workflow status is failed
WF_STATUS=$(sqlite3 /tmp/e2e_wf_12.db "SELECT status FROM workflow_runs LIMIT 1" 2>/dev/null || echo "")
[ "$WF_STATUS" = "failed" ] && pass "timeout: workflow run status is 'failed'" || fail "Timeout status" "got '$WF_STATUS'"

########################################
echo ""
echo "==============================="
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC}"
echo "==============================="

[ $FAIL -eq 0 ] && exit 0 || exit 1
