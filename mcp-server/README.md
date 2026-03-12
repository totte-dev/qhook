# qhook MCP Server

An [MCP (Model Context Protocol)](https://modelcontextprotocol.io) server that wraps qhook's REST API, allowing AI assistants to interact with your qhook webhook gateway.

## Prerequisites

- Node.js 18+ (uses built-in `fetch`)
- A running qhook instance

## Installation

```bash
cd mcp-server
npm install
npm run build
```

## Configuration

| Environment Variable | Description | Default |
|---|---|---|
| `QHOOK_URL` | qhook server URL | `http://localhost:8888` |
| `QHOOK_API_TOKEN` | API auth token (if `api.auth_token` is configured) | _(none)_ |

## Usage

### Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "qhook": {
      "command": "node",
      "args": ["mcp-server/dist/index.js"],
      "env": {
        "QHOOK_URL": "http://localhost:8888"
      }
    }
  }
}
```

### Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%/Claude/claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "qhook": {
      "command": "node",
      "args": ["/absolute/path/to/mcp-server/dist/index.js"],
      "env": {
        "QHOOK_URL": "http://localhost:8888",
        "QHOOK_API_TOKEN": "your-token-here"
      }
    }
  }
}
```

## Available Tools

| Tool | Description | qhook API |
|---|---|---|
| `health_check` | Check server health and queue depth | `GET /health` |
| `send_event` | Send an event to a configured source | `POST /events/{source}/{event_type}` |
| `get_event` | Get event details with jobs and workflow runs | `GET /api/events/{event_id}` |
| `get_job` | Get job details with delivery attempts | `GET /api/jobs/{job_id}` |
| `list_events` | Look up an event by ID | `GET /api/events/{event_id}` |
| `list_jobs` | List jobs for a given event | `GET /api/events/{event_id}` (jobs field) |
| `retry_job` | Retry a failed job (CLI only, not available via REST) | _N/A_ |

### Tool Details

#### `health_check`
No inputs required. Returns server status and current queue depth.

#### `send_event`
- `source` (required) — Source name as defined in qhook config (must be `type: event`)
- `event_type` (required) — Event type identifier (e.g., `order.created`)
- `payload` (required) — JSON object to send as the event body

#### `get_event`
- `event_id` (required) — Event ID (ULID)

#### `get_job`
- `job_id` (required) — Job ID (ULID)
- `include_attempts` (optional, default: `true`) — Include delivery attempt history

#### `list_events`
- `event_id` (required) — Event ID (ULID) to look up

> **Note:** qhook does not currently have a list/search endpoint for events. This tool retrieves a single event by ID. A full list endpoint may be added in a future release.

#### `list_jobs`
- `event_id` (required) — Event ID (ULID) whose jobs to list

> **Note:** Jobs are retrieved from the event's response. There is no standalone jobs list endpoint.

#### `retry_job`
- `job_id` (required) — Job ID (ULID) to retry

> **Note:** Retry is a CLI-only feature. This tool will return instructions to use the CLI: `qhook jobs retry <job_id>`.
