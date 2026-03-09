"""Mock HTTP server for workflow E2E testing.
Returns JSON responses based on path, enabling step chaining verification."""
import json
import sys
from http.server import HTTPServer, BaseHTTPRequestHandler
from threading import Lock

received = []
lock = Lock()
# Optional: fail specific paths with specific status codes
fail_paths = {}  # path -> (status_code, count) - fail first N requests to this path


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length).decode("utf-8")

        with lock:
            entry = {
                "path": self.path,
                "headers": dict(self.headers),
                "body": body,
            }
            received.append(entry)

            # Count requests to this path
            path_count = sum(1 for r in received if r["path"] == self.path)

        # Check if this path should fail
        if self.path in fail_paths:
            fail_code, fail_n = fail_paths[self.path]
            if path_count <= fail_n:
                self.send_response(fail_code)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({"error": f"Intentional {fail_code}"}).encode())
                return

        # Return JSON response based on path
        try:
            parsed = json.loads(body) if body else {}
        except json.JSONDecodeError:
            parsed = {}

        # Generate response based on the step path
        if "/validate" in self.path:
            response = {"valid": True, "risk_score": 0.05}
        elif "/fulfill" in self.path:
            order_id = parsed.get("order_id", parsed.get("id", "unknown"))
            response = {"fulfilled": True, "order_id": order_id, "tracking": "TRK-001"}
        elif "/notify" in self.path or "/slack" in self.path:
            response = {"notified": True}
        elif "/bad-request" in self.path:
            response = {"handled": True, "type": "bad_request"}
        elif "/alert" in self.path:
            response = {"alerted": True}
        else:
            # Echo back what we received with a marker
            response = {"received": True, "input": parsed}

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(response).encode())

    def do_GET(self):
        if self.path == "/received":
            with lock:
                data = json.dumps(received)
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(data.encode())
        elif self.path == "/count":
            with lock:
                count = len(received)
            self.send_response(200)
            self.end_headers()
            self.wfile.write(str(count).encode())
        elif self.path.startswith("/count/"):
            # Count requests to a specific path prefix
            prefix = self.path[7:]  # strip "/count/"
            with lock:
                count = sum(1 for r in received if prefix in r["path"])
            self.send_response(200)
            self.end_headers()
            self.wfile.write(str(count).encode())
        elif self.path == "/reset":
            with lock:
                received.clear()
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"reset")
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, format, *args):
        pass  # Suppress logs


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9999

    # Parse fail paths: path:status:count (e.g., /validate:500:2)
    for arg in sys.argv[2:]:
        parts = arg.split(":")
        if len(parts) == 3:
            fail_paths[parts[0]] = (int(parts[1]), int(parts[2]))

    server = HTTPServer(("127.0.0.1", port), Handler)
    print(f"Mock workflow server on :{port} (fail_paths={fail_paths})")
    sys.stdout.flush()
    server.serve_forever()
