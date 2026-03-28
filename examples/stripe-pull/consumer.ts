/**
 * Pull-mode consumer for qhook queues.
 * Polls for messages, processes them, and acks/nacks.
 * Requires Node.js 18+ (built-in fetch).
 */

const QHOOK_URL = "http://localhost:8888";
const QUEUE = "payments";

let running = true;

process.on("SIGINT", () => {
  console.log("\nShutting down...");
  running = false;
});

interface QueueMessage {
  id: string;
  event_id: string;
  event_type: string;
  payload: Record<string, unknown>;
  headers: Record<string, string> | null;
  attempt: number;
  created_at: string;
}

async function poll(wait = 10, batch = 1): Promise<QueueMessage[]> {
  try {
    const resp = await fetch(
      `${QHOOK_URL}/api/queues/${QUEUE}/messages?wait=${wait}s&batch=${batch}`,
      { signal: AbortSignal.timeout((wait + 5) * 1000) },
    );
    const data = (await resp.json()) as { messages: QueueMessage[] };
    return data.messages ?? [];
  } catch (err) {
    console.error(`[error] poll failed: ${err}`);
    return [];
  }
}

async function ack(ids: string[]) {
  const resp = await fetch(`${QHOOK_URL}/api/queues/${QUEUE}/ack`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ids }),
  });
  const data = (await resp.json()) as { acked: number };
  console.log(`  acked ${data.acked} message(s)`);
}

async function nack(ids: string[]) {
  const resp = await fetch(`${QHOOK_URL}/api/queues/${QUEUE}/nack`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ids }),
  });
  const data = (await resp.json()) as { retried: number; dead: number };
  console.log(`  nacked: retried=${data.retried}, dead=${data.dead}`);
}

function processMessage(msg: QueueMessage): boolean {
  const { event_type, payload } = msg;

  if (event_type === "checkout.session.completed") {
    console.log(
      `[payment] completed: id=${payload.id}, amount=${payload.amount_total}, customer=${payload.customer}`,
    );
    return true;
  }

  if (event_type === "charge.failed") {
    console.log(
      `[charge] failed: id=${payload.id}, failure=${payload.failure_message}`,
    );
    return true;
  }

  console.log(`[unknown] event_type=${event_type}`);
  return false;
}

async function main() {
  console.log(`Polling queue '${QUEUE}' at ${QHOOK_URL} (Ctrl+C to stop)`);

  while (running) {
    const messages = await poll(10, 5);
    if (messages.length === 0) continue;

    const okIds: string[] = [];
    const failIds: string[] = [];

    for (const msg of messages) {
      try {
        if (processMessage(msg)) {
          okIds.push(msg.id);
        } else {
          failIds.push(msg.id);
        }
      } catch (err) {
        console.error(`  [error] ${err}`);
        failIds.push(msg.id);
      }
    }

    if (okIds.length > 0) await ack(okIds);
    if (failIds.length > 0) await nack(failIds);
  }
}

main();
