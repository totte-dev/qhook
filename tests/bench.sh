#!/bin/bash
# qhook benchmark: measures receive RPS and end-to-end delivery throughput.
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

N=${1:-1000}          # total requests
C=${2:-50}            # concurrency

QHOOK_PORT=19201
MOCK_PORT=19011

now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }

PIDS=()
cleanup() {
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    rm -f /tmp/bench_qhook.yaml /tmp/bench_qhook.db
}
trap cleanup EXIT

echo "=== qhook benchmark (n=$N, c=$C) ==="
echo ""

# Build release
echo "Building (release)..."
cargo build --release --quiet 2>&1 | grep -v warning || true

# Start mock server
python3 "$SCRIPT_DIR/mock_server.py" $MOCK_PORT &
PIDS+=($!)
sleep 0.3

# Config
cat > /tmp/bench_qhook.yaml <<EOF
database:
  driver: sqlite
  url: "sqlite:/tmp/bench_qhook.db?mode=rwc"
server:
  port: $QHOOK_PORT
sources:
  app:
    type: event
handlers:
  bench:
    source: app
    events: [bench.test]
    url: http://127.0.0.1:$MOCK_PORT/job
    retry: { max: 1 }
EOF

# Start qhook
./target/release/qhook start --config /tmp/bench_qhook.yaml &>/dev/null &
PIDS+=($!)
sleep 1

# --- Benchmark 1: Receive RPS ---
echo "--- 1. Receive RPS (HTTP POST /events/app/bench.test) ---"
echo ""

ab -n "$N" -c "$C" -T "application/json" \
   -p <(echo '{"id":"bench","data":"hello"}') \
   "http://127.0.0.1:$QHOOK_PORT/events/app/bench.test" 2>/dev/null \
   | grep -E "(Requests per second|Time taken|Complete requests|Failed requests|50%|95%|99%)"

echo ""

# --- Benchmark 2: End-to-end delivery throughput ---
echo "--- 2. Delivery throughput ---"

echo -n "Waiting for $N deliveries..."
START=$(now_ms)
TIMEOUT=120

while true; do
    COUNT=$(curl -s "http://127.0.0.1:$MOCK_PORT/count" 2>/dev/null || echo 0)
    if [ "$COUNT" -ge "$N" ] 2>/dev/null; then
        break
    fi
    NOW=$(now_ms)
    ELAPSED_S=$(( (NOW - START) / 1000 ))
    if [ "$ELAPSED_S" -ge "$TIMEOUT" ]; then
        echo ""
        echo "  TIMEOUT: Only $COUNT/$N delivered in ${TIMEOUT}s"
        break
    fi
    sleep 0.2
done

END=$(now_ms)
DURATION_MS=$((END - START))
DELIVERED=$(curl -s "http://127.0.0.1:$MOCK_PORT/count" 2>/dev/null || echo 0)

echo " done"
echo ""
echo "  Delivered: $DELIVERED / $N"
echo "  Time: ${DURATION_MS}ms"

if [ "$DURATION_MS" -gt 0 ]; then
    TPS=$(python3 -c "print(f'{$DELIVERED * 1000 / $DURATION_MS:.1f}')")
    echo "  Throughput: ${TPS} deliveries/sec"
fi

echo ""
echo "=== Done ==="
