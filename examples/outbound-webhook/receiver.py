"""
Customer webhook receiver that verifies Standard Webhooks signatures.
Demonstrates how your customers verify outbound webhook deliveries.
No dependencies beyond Python stdlib.
"""
from http.server import HTTPServer, BaseHTTPRequestHandler
import base64
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

        # Extract Standard Webhooks headers
        signature = self.headers.get("webhook-signature", "")
        timestamp = self.headers.get("webhook-timestamp", "")
        msg_id = self.headers.get("webhook-id", "")
        # Supplementary qhook headers
        event_type = self.headers.get("X-Qhook-Event-Type", "")
        event_id = self.headers.get("X-Qhook-Event-ID", "")

        # Verify signature per Standard Webhooks spec
        verified = False
        if SIGNING_SECRET and signature.startswith("v1,"):
            # Decode the whsec_ secret to raw key bytes
            secret_b64 = SIGNING_SECRET.removeprefix("whsec_")
            key_bytes = base64.b64decode(secret_b64)
            # Signed content: {msg_id}.{timestamp}.{body}
            signed_content = f"{msg_id}.{timestamp}.".encode() + body
            expected = base64.b64encode(
                hmac.new(key_bytes, signed_content, hashlib.sha256).digest()
            ).decode()
            verified = hmac.compare_digest(signature[3:], expected)

        payload = json.loads(body) if body else {}
        status = "VERIFIED" if verified else ("UNVERIFIED" if SIGNING_SECRET else "NO_SECRET")

        print(f"[{status}] event_type={event_type} event_id={event_id} "
              f"msg_id={msg_id} payload={json.dumps(payload)}")

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
