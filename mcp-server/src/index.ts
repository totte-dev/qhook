#!/usr/bin/env node

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const QHOOK_URL = process.env.QHOOK_URL || "http://localhost:8888";
const QHOOK_API_TOKEN = process.env.QHOOK_API_TOKEN;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function headers(): Record<string, string> {
  const h: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (QHOOK_API_TOKEN) {
    h["Authorization"] = `Bearer ${QHOOK_API_TOKEN}`;
  }
  return h;
}

async function qhookFetch(
  path: string,
  init?: RequestInit,
): Promise<{ ok: boolean; status: number; body: unknown }> {
  const url = `${QHOOK_URL}${path}`;
  try {
    const res = await fetch(url, {
      ...init,
      headers: { ...headers(), ...(init?.headers as Record<string, string>) },
    });
    const text = await res.text();
    let body: unknown;
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
    return { ok: res.ok, status: res.status, body };
  } catch (err) {
    const message =
      err instanceof Error ? err.message : "Unknown error connecting to qhook";
    return {
      ok: false,
      status: 0,
      body: {
        error: `Failed to connect to qhook at ${QHOOK_URL}: ${message}`,
      },
    };
  }
}

function textResult(data: unknown) {
  return {
    content: [{ type: "text" as const, text: JSON.stringify(data, null, 2) }],
  };
}

function errorResult(message: string) {
  return {
    content: [{ type: "text" as const, text: message }],
    isError: true,
  };
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

const server = new McpServer({
  name: "qhook",
  version: "0.1.0",
});

// 1. health_check ─────────────────────────────────────────────────────────────

server.tool(
  "health_check",
  "Check qhook server health and queue depth",
  {},
  async () => {
    const res = await qhookFetch("/health");
    if (!res.ok) {
      return errorResult(
        `Health check failed (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    return textResult(res.body);
  },
);

// 2. send_event ───────────────────────────────────────────────────────────────

server.tool(
  "send_event",
  "Send an event to qhook (POST /events/{source}/{event_type})",
  {
    source: z.string().describe("Source name (must be type: event in config)"),
    event_type: z.string().describe("Event type identifier (e.g. order.created)"),
    payload: z
      .record(z.unknown())
      .describe("JSON payload to send as the event body"),
  },
  async ({ source, event_type, payload }) => {
    const res = await qhookFetch(`/events/${encodeURIComponent(source)}/${encodeURIComponent(event_type)}`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
    if (!res.ok) {
      return errorResult(
        `Failed to send event (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    return textResult(res.body);
  },
);

// 3. get_event ────────────────────────────────────────────────────────────────

server.tool(
  "get_event",
  "Get event details with associated jobs and workflow runs (GET /api/events/{event_id})",
  {
    event_id: z.string().describe("Event ID (ULID)"),
  },
  async ({ event_id }) => {
    const res = await qhookFetch(`/api/events/${encodeURIComponent(event_id)}`);
    if (!res.ok) {
      return errorResult(
        `Failed to get event (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    return textResult(res.body);
  },
);

// 4. get_job ──────────────────────────────────────────────────────────────────

server.tool(
  "get_job",
  "Get job details with optional delivery attempts (GET /api/jobs/{job_id})",
  {
    job_id: z.string().describe("Job ID (ULID)"),
    include_attempts: z
      .boolean()
      .optional()
      .default(true)
      .describe("Include delivery attempts in response (default: true)"),
  },
  async ({ job_id, include_attempts }) => {
    const params = include_attempts ? "?include_attempts=true" : "";
    const res = await qhookFetch(
      `/api/jobs/${encodeURIComponent(job_id)}${params}`,
    );
    if (!res.ok) {
      return errorResult(
        `Failed to get job (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    return textResult(res.body);
  },
);

// 5. list_events ──────────────────────────────────────────────────────────────
// NOTE: qhook does not currently expose a GET /api/events endpoint.
// The CLI `qhook events` command queries the database directly.
// This tool fetches a single event by ID as a workaround. When a list
// endpoint is added to the REST API, this tool should be updated.

server.tool(
  "list_events",
  "Look up events. Currently fetches a single event by ID (qhook REST API does not yet have a list endpoint). Provide event_id to retrieve event details.",
  {
    event_id: z.string().describe("Event ID (ULID) to look up"),
  },
  async ({ event_id }) => {
    const res = await qhookFetch(`/api/events/${encodeURIComponent(event_id)}`);
    if (!res.ok) {
      return errorResult(
        `Failed to get event (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    return textResult(res.body);
  },
);

// 6. list_jobs ────────────────────────────────────────────────────────────────
// NOTE: Like list_events, qhook does not expose a GET /api/jobs list endpoint.
// Jobs are available nested inside event responses. This tool retrieves jobs
// for a specific event.

server.tool(
  "list_jobs",
  "List jobs for a given event. Retrieves the event and returns its associated jobs.",
  {
    event_id: z
      .string()
      .describe("Event ID (ULID) to list jobs for"),
  },
  async ({ event_id }) => {
    const res = await qhookFetch(`/api/events/${encodeURIComponent(event_id)}`);
    if (!res.ok) {
      return errorResult(
        `Failed to get jobs (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    const event = res.body as Record<string, unknown>;
    return textResult(event.jobs ?? []);
  },
);

// 7. retry_job ────────────────────────────────────────────────────────────────
// NOTE: qhook does not expose a retry endpoint in the REST API.
// Retry is a CLI-only feature (qhook jobs retry) that operates on the DB directly.
// This tool documents the limitation and suggests using the CLI.

server.tool(
  "retry_job",
  "Retry a failed/dead job. NOTE: qhook does not currently expose a retry REST endpoint. Use the qhook CLI instead: `qhook jobs retry <job_id>`",
  {
    job_id: z.string().describe("Job ID (ULID) to retry"),
  },
  async ({ job_id }) => {
    return errorResult(
      `Retry is not available via the qhook REST API. ` +
        `Use the CLI instead: qhook jobs retry ${job_id}\n\n` +
        `You can inspect the job with the get_job tool first to check its status.`,
    );
  },
);

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
