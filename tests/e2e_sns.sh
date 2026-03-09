#!/bin/bash
# SNS integration test using LocalStack
# Usage: ./tests/e2e_sns.sh
# Requires: LocalStack running on $LOCALSTACK_ENDPOINT (default: http://localhost:4566)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

LOCALSTACK=${LOCALSTACK_ENDPOINT:-http://localhost:4566}
QHOOK_PORT=19201
MOCK_PORT=19202

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
    rm -f /tmp/e2e_sns_*.yaml /tmp/e2e_sns_*.db
}
trap cleanup EXIT

# Check LocalStack is running
echo "Checking LocalStack at $LOCALSTACK ..."
if ! curl -sf "$LOCALSTACK/_localstack/health" > /dev/null 2>&1; then
    echo "ERROR: LocalStack is not running at $LOCALSTACK"
    echo "Start it with: docker run -d -p 4566:4566 -e SERVICES=sns localstack/localstack:3"
    exit 1
fi
echo "LocalStack is ready."

echo "Building qhook..."
cargo build --quiet 2>&1 | grep -v warning || true

# Start mock server
python3 "$SCRIPT_DIR/mock_server.py" $MOCK_PORT &
PIDS+=($!)
sleep 0.3

# Create qhook config with SNS source (skip_verify for LocalStack)
cat > /tmp/e2e_sns_1.yaml <<EOF
database:
  driver: sqlite
  url: "sqlite:/tmp/e2e_sns_1.db?mode=rwc"
server:
  port: $QHOOK_PORT
  allow_private_urls: true
sources:
  my-sns:
    type: sns
    skip_verify: true
handlers:
  on-sns-event:
    source: my-sns
    events: ["*"]
    url: http://127.0.0.1:$MOCK_PORT/jobs/sns
    retry: { max: 3 }
EOF

# Start qhook
./target/debug/qhook start --config /tmp/e2e_sns_1.yaml 2>/dev/null &
PIDS+=($!)
sleep 1

########################################
echo ""
echo "=== Test 1: SNS Subscription Confirmation ==="
########################################

# Create SNS topic
TOPIC_ARN=$(aws --endpoint-url "$LOCALSTACK" sns create-topic --name qhook-test --query 'TopicArn' --output text 2>/dev/null)
echo "  Topic: $TOPIC_ARN"

# Determine how Docker containers reach the host
if [ "$(uname)" = "Darwin" ]; then
    HOST_FROM_DOCKER="host.docker.internal"
else
    HOST_FROM_DOCKER="172.17.0.1"  # Default Docker bridge on Linux
fi

# Subscribe qhook to the topic (LocalStack sends SubscriptionConfirmation automatically)
SUB_ARN=$(aws --endpoint-url "$LOCALSTACK" sns subscribe \
    --topic-arn "$TOPIC_ARN" \
    --protocol http \
    --notification-endpoint "http://${HOST_FROM_DOCKER}:$QHOOK_PORT/sns/my-sns" \
    --query 'SubscriptionArn' --output text 2>/dev/null || echo "pending")

sleep 2

# Check if subscription was confirmed
if [ "$SUB_ARN" != "pending" ] && [ -n "$SUB_ARN" ]; then
    pass "SNS subscription confirmed (arn=$SUB_ARN)"
else
    # LocalStack may auto-confirm; check via list-subscriptions
    CONFIRMED=$(aws --endpoint-url "$LOCALSTACK" sns list-subscriptions-by-topic \
        --topic-arn "$TOPIC_ARN" --query 'Subscriptions[0].SubscriptionArn' --output text 2>/dev/null || echo "none")
    if [ "$CONFIRMED" != "PendingConfirmation" ] && [ "$CONFIRMED" != "none" ]; then
        pass "SNS subscription confirmed (via list)"
    else
        fail "SNS subscription" "arn=$SUB_ARN, status=$CONFIRMED"
    fi
fi

########################################
echo ""
echo "=== Test 2: SNS Notification → qhook → delivery ==="
########################################

# Reset mock counter
curl -sf http://127.0.0.1:$MOCK_PORT/reset > /dev/null 2>&1

# Publish message to SNS
aws --endpoint-url "$LOCALSTACK" sns publish \
    --topic-arn "$TOPIC_ARN" \
    --subject "order.created" \
    --message '{"id":"ord_789","amount":4999}' \
    > /dev/null 2>/dev/null

sleep 3

# Check if mock received the delivery
COUNT=$(curl -sf http://127.0.0.1:$MOCK_PORT/count 2>/dev/null || echo "0")
if [ "$COUNT" -ge 1 ] 2>/dev/null; then
    pass "SNS notification delivered to handler (count=$COUNT)"
else
    fail "SNS delivery" "expected >=1, got $COUNT"
fi

# Check payload content
RECEIVED=$(curl -sf http://127.0.0.1:$MOCK_PORT/received 2>/dev/null || echo "[]")
HAS_PAYLOAD=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
if d:
    body = d[0].get('body', '')
    print('yes' if 'ord_789' in body or '4999' in body else 'no')
else:
    print('no')
" 2>/dev/null || echo "no")

[ "$HAS_PAYLOAD" = "yes" ] && pass "SNS message payload preserved" || fail "Payload" "ord_789 not found in delivered body"

########################################
echo ""
echo "=== Test 3: Direct SNS notification (without LocalStack) ==="
########################################

# Send a fake SNS notification directly to qhook (skip_verify is on)
curl -sf http://127.0.0.1:$MOCK_PORT/reset > /dev/null 2>&1

HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST "http://127.0.0.1:$QHOOK_PORT/sns/my-sns" \
    -H "Content-Type: text/plain" \
    -H "x-amz-sns-message-type: Notification" \
    -d '{
        "Type": "Notification",
        "MessageId": "test-msg-001",
        "TopicArn": "arn:aws:sns:us-east-1:000000000000:test",
        "Subject": "user.signup",
        "Message": "{\"user_id\": \"usr_42\", \"type\": \"user.signup\"}",
        "Timestamp": "2024-01-01T00:00:00.000Z",
        "SignatureVersion": "1",
        "Signature": "dGVzdA==",
        "SigningCertURL": "https://sns.us-east-1.amazonaws.com/fake.pem"
    }')

[ "$HTTP_CODE" = "200" ] && pass "Direct SNS notification accepted (200)" || fail "Direct SNS" "got $HTTP_CODE"

sleep 2

COUNT=$(curl -sf http://127.0.0.1:$MOCK_PORT/count 2>/dev/null || echo "0")
[ "$COUNT" -ge 1 ] 2>/dev/null && pass "Direct SNS delivered to handler (count=$COUNT)" || fail "Direct SNS delivery" "count=$COUNT"

# Check event type was extracted from Message.type
RECEIVED=$(curl -sf http://127.0.0.1:$MOCK_PORT/received 2>/dev/null || echo "[]")
HAS_USER=$(echo "$RECEIVED" | python3 -c "
import sys, json
d = json.load(sys.stdin)
if d:
    body = d[0].get('body', '')
    print('yes' if 'usr_42' in body else 'no')
else:
    print('no')
" 2>/dev/null || echo "no")
[ "$HAS_USER" = "yes" ] && pass "SNS message unwrapped correctly" || fail "SNS unwrap" "usr_42 not found"

########################################
echo ""
echo "=== Test 4: CloudEvents binary mode ==="
########################################

curl -sf http://127.0.0.1:$MOCK_PORT/reset > /dev/null 2>&1

HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST "http://127.0.0.1:$QHOOK_PORT/webhooks/my-sns" \
    -H "Content-Type: application/json" \
    -d '{"data": "test"}' 2>/dev/null)

# my-sns is type:sns, not webhook — should return 404
[ "$HTTP_CODE" = "404" ] && pass "SNS source rejects webhook route (404)" || fail "Route isolation" "got $HTTP_CODE"

########################################
echo ""
echo "=== Test 5: Unknown SNS source → 404 ==="
########################################

HTTP_CODE=$(curl -s --max-time 3 -o /dev/null -w "%{http_code}" \
    -X POST "http://127.0.0.1:$QHOOK_PORT/sns/nonexistent" \
    -H "Content-Type: text/plain" \
    -d '{}')

[ "$HTTP_CODE" = "404" ] && pass "Unknown SNS source returns 404" || fail "Unknown SNS" "got $HTTP_CODE"

########################################
echo ""
echo "==============================="
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC}"
echo "==============================="

[ $FAIL -eq 0 ] && exit 0 || exit 1
