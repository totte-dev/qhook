# GitHub Webhook Example

Receive GitHub `push` and `pull_request` events with signature verification, routing each to a different handler.

## Architecture

```
GitHub  --->  qhook (HMAC-SHA256 verify)  --->  deploy handler
                                           --->  notify handler
```

- Push to `main` triggers deployment
- Pull request events trigger notification

## Getting Started

```bash
docker compose up
```

qhook runs on `localhost:8888`, the handler app runs internally.

## Testing Locally

Since we don't have a real GitHub webhook, send test events using the internal event API:

**Push event:**

```bash
curl -X POST http://localhost:8888/events/push \
  -H "Content-Type: application/json" \
  -d '{
    "ref": "refs/heads/main",
    "repository": {"full_name": "myorg/myapp"},
    "pusher": {"name": "alice"},
    "head_commit": {"message": "fix: resolve timeout bug"}
  }'
```

**Pull request event:**

```bash
curl -X POST http://localhost:8888/events/pull_request \
  -H "Content-Type: application/json" \
  -d '{
    "action": "opened",
    "number": 42,
    "pull_request": {
      "title": "Add caching layer",
      "user": {"login": "bob"},
      "html_url": "https://github.com/myorg/myapp/pull/42"
    }
  }'
```

## Expected Output

```
[deploy] push to main by alice: "fix: resolve timeout bug"
[notify] PR #42 opened by bob: "Add caching layer"
```

## Production Setup

In production, configure the real GitHub webhook:

1. Set `GITHUB_WEBHOOK_SECRET` in your environment
2. In GitHub repo settings, add webhook URL: `https://your-host/webhooks/github`
3. Set content type to `application/json` and paste the same secret
4. qhook verifies every request using `X-Hub-Signature-256`

## What This Shows

- **Webhook signature verification** (GitHub HMAC-SHA256)
- **Event routing** -- different event types go to different handlers
- **Fan-out** -- one source, multiple handlers
