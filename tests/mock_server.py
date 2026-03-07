"""Simple mock HTTP server for E2E testing."""
import json
import sys
from http.server import HTTPServer, BaseHTTPRequestHandler
from threading import Lock

received = []
lock = Lock()
# Default: return 200. Set via command line arg.
response_code = 200
# After N failures, start returning 200 (for retry testing)
fail_count = 0
max_failures = 0


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
            count = len(received)

        global fail_count, max_failures, response_code
        if max_failures > 0 and count <= max_failures:
            self.send_response(500)
            self.end_headers()
            self.wfile.write(b"Intentional failure")
        else:
            self.send_response(response_code)
            self.end_headers()
            self.wfile.write(b"OK")

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
    if len(sys.argv) > 2:
        max_failures = int(sys.argv[2])

    server = HTTPServer(("127.0.0.1", port), Handler)
    print(f"Mock server on :{port} (fail first {max_failures} requests)")
    sys.stdout.flush()
    server.serve_forever()
