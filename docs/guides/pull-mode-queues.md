---
layout: default
title: Pull-Mode Queues
---

# Pull-Mode Queues

## Overview

By default, qhook operates in **push mode**: when an event arrives, it delivers the payload to a configured HTTP endpoint. Pull-mode queues invert this -- your consumer polls qhook for messages and explicitly acknowledges them after processing.

**Use push handlers when** the consumer is always online and you want minimal latency.

**Use pull-mode queues when:**

- The consumer runs on a schedule (cron jobs, batch workers).
- You want backpressure control -- consumers pull only what they can handle.
- The downstream service is behind a firewall or NAT with no public endpoint.
- You need at-least-once semantics with explicit acknowledgment.

## Configuration

Define queues in the `queues:` section of `qhook.yaml`. Each queue subscribes to a source and optionally filters by event type.

```yaml
sources:
  stripe:
    path: /webhook/stripe
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}

queues:
  billing:
    source: stripe
    events: [invoice.paid, invoice.payment_failed]
    filter: "$.data.object.amount_due >= 1000"
    visibility_timeout: 60s
    max_attempts: 5
    api_key: ${QUEUE_API_KEY}
```

### Queue options

| Field | Default | Description |
|-------|---------|-------------|
| `source` | (required) | Source name to subscribe to. |
| `events` | `[]` (all) | Event types to accept. Empty means all events from the source. |
| `filter` | none | JSONPath filter expression. Only matching events enter the queue. |
| `idempotency_key` | none | JSONPath for deduplication key extraction. |
| `visibility_timeout` | `30s` | How long a delivered message stays invisible before re-queuing. |
| `max_attempts` | none | Max delivery attempts before moving to the dead-letter queue. |
| `api_key` | none | Per-queue Bearer token. If unset, no authentication is required. |

## API Reference

### List queues

```
GET /api/queues
Authorization: Bearer <management-api-key>
```

Response:

```json
{
  "queues": [
    {
      "name": "billing",
      "source": "stripe",
      "events": ["invoice.paid", "invoice.payment_failed"],
      "visibility_timeout": "60s",
      "depth": 42
    }
  ]
}
```

### Receive messages (long-polling)

```
GET /api/queues/{name}/messages?wait=10s&batch=5
Authorization: Bearer <queue-api-key>
```

| Parameter | Default | Description |
|-----------|---------|-------------|
| `wait` | `0s` | Long-poll duration. Max `30s`. Returns immediately if messages are available. |
| `batch` | `1` | Number of messages to receive. Range: 1--100. |

Response:

```json
{
  "messages": [
    {
      "id": "01J...",
      "event_id": "01J...",
      "event_type": "invoice.paid",
      "payload": { "...": "..." },
      "headers": { "content-type": "application/json" },
      "attempt": 1,
      "created_at": "2026-03-28T12:00:00.000"
    }
  ]
}
```

An empty `{"messages": []}` is returned when no messages are available after the wait period expires.

### Acknowledge messages

```
POST /api/queues/{name}/ack
Authorization: Bearer <queue-api-key>
Content-Type: application/json

{"ids": ["01J..."]}
```

Response:

```json
{"acked": 1}
```

Acknowledged messages are permanently removed from the queue.

### Negative-acknowledge messages

```
POST /api/queues/{name}/nack
Authorization: Bearer <queue-api-key>
Content-Type: application/json

{"ids": ["01J..."]}
```

Response:

```json
{"retried": 1, "dead": 0}
```

Nacked messages are re-queued for retry. If `max_attempts` is set and the message has exceeded it, the message moves to the dead-letter queue instead (`dead` count increments).

## CLI Commands

```bash
# List all queues with job counts
qhook queues list

# Show detailed stats and recent messages for a queue
qhook queues inspect billing --limit 20

# Peek at the next message without consuming it
qhook queues peek billing

# List dead-letter messages
qhook queues dlq billing --limit 50

# Retry all dead-letter messages
qhook queues retry billing

# Retry a specific dead-letter message
qhook queues retry billing --id 01J...

# Delete all jobs for a queue
qhook queues drain billing --force
```

All commands accept `-c <path>` to specify a config file (default: `qhook.yaml`).

## Consumer Snippets

### Python

```python
import requests
import time

BASE = "http://localhost:8080"
QUEUE = "billing"
HEADERS = {"Authorization": "Bearer my-secret-key"}

while True:
    resp = requests.get(
        f"{BASE}/api/queues/{QUEUE}/messages",
        params={"wait": "10s", "batch": 5},
        headers=HEADERS,
        timeout=15,
    )
    messages = resp.json().get("messages", [])
    if not messages:
        continue

    ack_ids = []
    for msg in messages:
        try:
            process(msg["payload"])
            ack_ids.append(msg["id"])
        except Exception as e:
            print(f"Failed to process {msg['id']}: {e}")
            requests.post(
                f"{BASE}/api/queues/{QUEUE}/nack",
                json={"ids": [msg["id"]]},
                headers=HEADERS,
            )

    if ack_ids:
        requests.post(
            f"{BASE}/api/queues/{QUEUE}/ack",
            json={"ids": ack_ids},
            headers=HEADERS,
        )
```

### TypeScript / Node.js

```typescript
const BASE = "http://localhost:8080";
const QUEUE = "billing";
const HEADERS = {
  Authorization: "Bearer my-secret-key",
  "Content-Type": "application/json",
};

async function consume() {
  while (true) {
    const res = await fetch(
      `${BASE}/api/queues/${QUEUE}/messages?wait=10s&batch=5`,
      { headers: HEADERS, signal: AbortSignal.timeout(15_000) }
    );
    const { messages } = await res.json();
    if (!messages.length) continue;

    const ackIds: string[] = [];
    for (const msg of messages) {
      try {
        await process(msg.payload);
        ackIds.push(msg.id);
      } catch (err) {
        console.error(`Failed ${msg.id}:`, err);
        await fetch(`${BASE}/api/queues/${QUEUE}/nack`, {
          method: "POST",
          headers: HEADERS,
          body: JSON.stringify({ ids: [msg.id] }),
        });
      }
    }

    if (ackIds.length) {
      await fetch(`${BASE}/api/queues/${QUEUE}/ack`, {
        method: "POST",
        headers: HEADERS,
        body: JSON.stringify({ ids: ackIds }),
      });
    }
  }
}

consume();
```

### Go

```go
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"time"
)

const (
	base  = "http://localhost:8080"
	queue = "billing"
	token = "Bearer my-secret-key"
)

type Message struct {
	ID        string          `json:"id"`
	EventType string          `json:"event_type"`
	Payload   json.RawMessage `json:"payload"`
	Attempt   int             `json:"attempt"`
}

func main() {
	client := &http.Client{Timeout: 15 * time.Second}

	for {
		url := fmt.Sprintf("%s/api/queues/%s/messages?wait=10s&batch=5", base, queue)
		req, _ := http.NewRequest("GET", url, nil)
		req.Header.Set("Authorization", token)

		resp, err := client.Do(req)
		if err != nil {
			fmt.Println("poll error:", err)
			time.Sleep(time.Second)
			continue
		}

		var result struct{ Messages []Message }
		json.NewDecoder(resp.Body).Decode(&result)
		resp.Body.Close()

		if len(result.Messages) == 0 {
			continue
		}

		var ackIDs []string
		for _, msg := range result.Messages {
			if err := process(msg.Payload); err != nil {
				postIDs(client, "nack", []string{msg.ID})
			} else {
				ackIDs = append(ackIDs, msg.ID)
			}
		}

		if len(ackIDs) > 0 {
			postIDs(client, "ack", ackIDs)
		}
	}
}

func postIDs(client *http.Client, action string, ids []string) {
	url := fmt.Sprintf("%s/api/queues/%s/%s", base, queue, action)
	body, _ := json.Marshal(map[string][]string{"ids": ids})
	req, _ := http.NewRequest("POST", url, bytes.NewReader(body))
	req.Header.Set("Authorization", token)
	req.Header.Set("Content-Type", "application/json")
	resp, err := client.Do(req)
	if err == nil {
		resp.Body.Close()
	}
}

func process(payload json.RawMessage) error {
	// your processing logic here
	return nil
}
```

### Ruby

```ruby
require "net/http"
require "json"
require "uri"

BASE = "http://localhost:8080"
QUEUE = "billing"
TOKEN = "Bearer my-secret-key"

def poll
  uri = URI("#{BASE}/api/queues/#{QUEUE}/messages?wait=10s&batch=5")
  req = Net::HTTP::Get.new(uri)
  req["Authorization"] = TOKEN

  http = Net::HTTP.new(uri.host, uri.port)
  http.read_timeout = 15

  loop do
    res = http.request(req)
    messages = JSON.parse(res.body).fetch("messages", [])
    next if messages.empty?

    ack_ids = []
    messages.each do |msg|
      begin
        process(msg["payload"])
        ack_ids << msg["id"]
      rescue => e
        warn "Failed #{msg['id']}: #{e}"
        post_ids("nack", [msg["id"]])
      end
    end

    post_ids("ack", ack_ids) unless ack_ids.empty?
  end
end

def post_ids(action, ids)
  uri = URI("#{BASE}/api/queues/#{QUEUE}/#{action}")
  req = Net::HTTP::Post.new(uri)
  req["Authorization"] = TOKEN
  req["Content-Type"] = "application/json"
  req.body = JSON.generate({ ids: ids })
  Net::HTTP.new(uri.host, uri.port).request(req)
end

def process(payload)
  # your processing logic here
end

poll
```

## Visibility Timeout

When a consumer receives a message, the message becomes **invisible** to other consumers for the duration of `visibility_timeout` (default: 30 seconds). This prevents duplicate processing in multi-consumer setups.

### How it works

1. Consumer calls `GET /api/queues/{name}/messages` -- the message is delivered and marked as in-flight.
2. The consumer has `visibility_timeout` seconds to process and acknowledge the message.
3. **If acknowledged (ack):** The message is permanently removed.
4. **If negative-acknowledged (nack):** The message is immediately re-queued for retry (or moved to DLQ if `max_attempts` is exceeded).
5. **If neither ack nor nack is sent:** After the visibility timeout expires, the message automatically becomes visible again and will be delivered to the next consumer that polls.

### Choosing a timeout value

Set `visibility_timeout` to at least 2x your expected processing time. If your handler takes 10 seconds on average, use `30s` or higher.

```yaml
queues:
  # Fast processing -- short timeout
  notifications:
    source: app
    visibility_timeout: 15s

  # Slow processing -- long timeout
  reports:
    source: app
    visibility_timeout: 5m
    max_attempts: 3
```

If a message is re-delivered due to timeout expiry, its `attempt` counter increments. Once `max_attempts` is reached, the next nack (or timeout expiry) moves it to the dead-letter queue, where you can inspect it with `qhook queues dlq` and retry with `qhook queues retry`.
