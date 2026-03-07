"""
Stripe Checkout backend (sample)
A simple Flask app that receives deliveries from qhook.
"""
from flask import Flask, request, jsonify
import logging

app = Flask(__name__)
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s")
log = logging.getLogger("app")


# Payment event endpoint
# qhook POSTs here when it receives a checkout.session.completed event
@app.route("/jobs/payment", methods=["POST"])
def handle_payment():
    payload = request.get_json(silent=True) or {}
    log.info("[payment] event received: id=%s, amount=%s",
             payload.get("id", "?"),
             payload.get("amount_total", "?"))
    # Your payment confirmation logic goes here
    return jsonify({"status": "ok"}), 200


# Fulfillment (shipping) endpoint
# The same event can be routed to multiple handlers -- a key qhook feature
@app.route("/jobs/fulfillment", methods=["POST"])
def handle_fulfillment():
    payload = request.get_json(silent=True) or {}
    log.info("[fulfillment] started: id=%s, customer=%s",
             payload.get("id", "?"),
             payload.get("customer", "?"))
    # Your inventory allocation / shipping logic goes here
    return jsonify({"status": "ok"}), 200


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=5000)
