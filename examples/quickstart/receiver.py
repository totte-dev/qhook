"""
Minimal HTTP receiver that logs deliveries from qhook.
No dependencies beyond Python stdlib.
"""
from http.server import HTTPServer, BaseHTTPRequestHandler
import json


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length)) if length else {}
        print(f"[order] received: id={body.get('id')}, "
              f"customer={body.get('customer')}, amount={body.get('amount')}")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def log_message(self, format, *args):
        pass  # suppress default access logs


if __name__ == "__main__":
    server = HTTPServer(("0.0.0.0", 9000), Handler)
    print("Receiver listening on :9000")
    server.serve_forever()
