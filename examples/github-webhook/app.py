"""
GitHub webhook handler (sample).
Receives push and PR events delivered by qhook.
"""
from flask import Flask, request, jsonify
import logging

app = Flask(__name__)
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s")
log = logging.getLogger("app")


@app.route("/deploy", methods=["POST"])
def handle_deploy():
    payload = request.get_json(silent=True) or {}
    pusher = payload.get("pusher", {}).get("name", "?")
    message = payload.get("head_commit", {}).get("message", "?")
    log.info('[deploy] push to main by %s: "%s"', pusher, message)
    # Your deployment logic here (e.g., trigger CI/CD pipeline)
    return jsonify({"status": "ok"}), 200


@app.route("/notify", methods=["POST"])
def handle_notify():
    payload = request.get_json(silent=True) or {}
    # When transform is applied, payload is the transformed shape
    text = payload.get("text", str(payload))
    log.info("[notify] %s", text)
    return jsonify({"status": "ok"}), 200


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=5000)
