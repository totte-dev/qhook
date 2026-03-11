"""
Customer webhook receiver that verifies qhook signatures.
Demonstrates how your customers verify outbound webhook deliveries.
No dependencies beyond Python stdlib.
"""
from http.server import HTTPServer, BaseHTTPRequestHandler
import hashlib
import hmac
import json
import sys


# Set this to the signing_secret returned when creating the endpoint
SIGNING_SECRET = sys.argv[1] if len(sys.argv) > 1 else ""


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)

        # Extract signature headers
        signature = self.headers.get("X-Qhook-Signature", "")
        timestamp = self.headers.get("X-Qhook-Timestamp", "")
        event_type = self.headers.get("X-Qhook-Event-Type", "")
        event_id = self.headers.get("X-Qhook-Event-ID", "")
        delivery_id = self.headers.get("X-Qhook-Delivery-ID", "")

        # Verify signature
        verified = False
        if SIGNING_SECRET and signature.startswith("v1="):
            expected = hmac.new(
                SIGNING_SECRET.encode(),
                f"{timestamp}.".encode() + body,
                hashlib.sha256,
            ).hexdigest()
            verified = hmac.compare_digest(signature[3:], expected)

        payload = json.loads(body) if body else {}
        status = "VERIFIED" if verified else ("UNVERIFIED" if SIGNING_SECRET else "NO_SECRET")

        print(f"[{status}] event_type={event_type} event_id={event_id} "
              f"delivery_id={delivery_id} payload={json.dumps(payload)}")

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def log_message(self, format, *args):
        pass  # suppress default access logs


if __name__ == "__main__":
    port = 9000
    server = HTTPServer(("0.0.0.0", port), Handler)
    print(f"Customer webhook receiver listening on :{port}")
    if SIGNING_SECRET:
        print(f"Verifying signatures with secret: {SIGNING_SECRET[:12]}...")
    else:
        print("No signing secret provided -- pass it as: python3 receiver.py whsec_...")
    server.serve_forever()
