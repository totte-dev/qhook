"""
Pull-mode consumer for qhook queues.
Polls for messages, processes them, and acks/nacks.
No dependencies beyond Python stdlib.
"""
import json
import signal
import sys
import urllib.request
import urllib.error

QHOOK_URL = "http://localhost:8888"
QUEUE = "payments"

running = True


def shutdown(sig, frame):
    global running
    print("\nShutting down...")
    running = False


signal.signal(signal.SIGINT, shutdown)


def poll(wait: int = 10, batch: int = 1) -> list:
    """Long-poll the queue for messages."""
    url = f"{QHOOK_URL}/api/queues/{QUEUE}/messages?wait={wait}s&batch={batch}"
    req = urllib.request.Request(url)
    try:
        with urllib.request.urlopen(req, timeout=wait + 5) as resp:
            data = json.loads(resp.read())
            return data.get("messages", [])
    except urllib.error.URLError as e:
        print(f"[error] poll failed: {e}")
        return []


def ack(ids: list[str]):
    """Acknowledge successfully processed messages."""
    url = f"{QHOOK_URL}/api/queues/{QUEUE}/ack"
    body = json.dumps({"ids": ids}).encode()
    req = urllib.request.Request(url, data=body, method="POST")
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req) as resp:
        data = json.loads(resp.read())
        print(f"  acked {data.get('acked', 0)} message(s)")


def nack(ids: list[str]):
    """Negative-acknowledge messages (retry or DLQ)."""
    url = f"{QHOOK_URL}/api/queues/{QUEUE}/nack"
    body = json.dumps({"ids": ids}).encode()
    req = urllib.request.Request(url, data=body, method="POST")
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req) as resp:
        data = json.loads(resp.read())
        print(f"  nacked: retried={data.get('retried', 0)}, dead={data.get('dead', 0)}")


def process(message: dict) -> bool:
    """Process a single message. Returns True on success."""
    event_type = message.get("event_type", "unknown")
    payload = message.get("payload", {})

    if event_type == "checkout.session.completed":
        print(f"[payment] completed: id={payload.get('id')}, "
              f"amount={payload.get('amount_total')}, "
              f"customer={payload.get('customer')}")
        return True

    if event_type == "charge.failed":
        print(f"[charge] failed: id={payload.get('id')}, "
              f"failure={payload.get('failure_message')}")
        return True

    print(f"[unknown] event_type={event_type}")
    return False


if __name__ == "__main__":
    print(f"Polling queue '{QUEUE}' at {QHOOK_URL} (Ctrl+C to stop)")
    while running:
        messages = poll(wait=10, batch=5)
        if not messages:
            continue

        ok_ids = []
        fail_ids = []
        for msg in messages:
            try:
                if process(msg):
                    ok_ids.append(msg["id"])
                else:
                    fail_ids.append(msg["id"])
            except Exception as e:
                print(f"  [error] {e}")
                fail_ids.append(msg["id"])

        if ok_ids:
            ack(ok_ids)
        if fail_ids:
            nack(fail_ids)
