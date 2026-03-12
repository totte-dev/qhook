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

server.tool(
  "list_events",
  "List events with optional filters. Returns events without payload (lightweight).",
  {
    source: z.string().optional().describe("Filter by source name"),
    event_type: z.string().optional().describe("Filter by event type"),
    since: z.string().optional().describe("Filter events created after this timestamp (ISO 8601)"),
    until: z.string().optional().describe("Filter events created before this timestamp (ISO 8601)"),
    limit: z.number().optional().describe("Max results (default: 50, max: 1000)"),
  },
  async ({ source, event_type, since, until, limit }) => {
    const params = new URLSearchParams();
    if (source) params.set("source", source);
    if (event_type) params.set("event_type", event_type);
    if (since) params.set("since", since);
    if (until) params.set("until", until);
    if (limit) params.set("limit", String(limit));
    const query = params.toString();
    const res = await qhookFetch(`/api/events${query ? `?${query}` : ""}`);
    if (!res.ok) {
      return errorResult(
        `Failed to list events (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    return textResult(res.body);
  },
);

// 6. list_jobs ────────────────────────────────────────────────────────────────

server.tool(
  "list_jobs",
  "List jobs with optional filters. Includes status, handler, attempt count.",
  {
    status: z.string().optional().describe("Filter by status (available, running, completed, dead)"),
    handler: z.string().optional().describe("Filter by handler name"),
    limit: z.number().optional().describe("Max results (default: 50, max: 1000)"),
  },
  async ({ status, handler, limit }) => {
    const params = new URLSearchParams();
    if (status) params.set("status", status);
    if (handler) params.set("handler", handler);
    if (limit) params.set("limit", String(limit));
    const query = params.toString();
    const res = await qhookFetch(`/api/jobs${query ? `?${query}` : ""}`);
    if (!res.ok) {
      return errorResult(
        `Failed to list jobs (HTTP ${res.status}): ${JSON.stringify(res.body)}`,
      );
    }
    return textResult(res.body);
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
