"""
Event handler (sample).
Receives filtered and transformed events from qhook.
"""
from flask import Flask, request, jsonify
import json
import logging

app = Flask(__name__)
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s")
log = logging.getLogger("app")


@app.route("/paid", methods=["POST"])
def handle_paid():
    payload = request.get_json(silent=True) or {}
    log.info("[paid] order %s from %s: %s %s",
             payload.get("id"), payload.get("customer"),
             payload.get("amount"), payload.get("currency"))
    return jsonify({"status": "ok"}), 200


@app.route("/slack", methods=["POST"])
def handle_slack():
    payload = request.get_json(silent=True) or {}
    log.info("[slack] %s", json.dumps(payload))
    return jsonify({"status": "ok"}), 200


@app.route("/audit", methods=["POST"])
def handle_audit():
    payload = request.get_json(silent=True) or {}
    log.info("[audit] %s", json.dumps(payload))
    return jsonify({"status": "ok"}), 200


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=5000)
