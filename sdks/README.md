# qhook SDKs

Auto-generated client libraries from the [OpenAPI spec](../docs/openapi.yaml).

## Languages

| Language | Package | Directory | Registry |
|----------|---------|-----------|----------|
| TypeScript | `qhook-client` | `sdks/typescript/` | npm |
| Python | `qhook-client` | `sdks/python/` | PyPI |

## Generate

```bash
# All languages
bash sdks/generate.sh all

# Single language
bash sdks/generate.sh typescript
bash sdks/generate.sh python
```

Requires [openapi-generator-cli](https://openapi-generator.tech/docs/installation):

```bash
npm install -g @openapitools/openapi-generator-cli
# or use npx (auto-detected by generate.sh)
```

## Usage

### TypeScript

```typescript
import { Configuration, IngestApi, ManagementApi } from "qhook-client";

const config = new Configuration({
  basePath: "http://localhost:8888",
  headers: { Authorization: "Bearer your-token" },
});

// Send an event
const ingest = new IngestApi(config);
const result = await ingest.sendEvent({
  source: "app",
  eventType: "order.created",
  body: { order_id: "ORD-123", total: 99.99 },
});
console.log(result.eventId, result.jobsCreated);

// Inspect an event
const mgmt = new ManagementApi(config);
const event = await mgmt.getEvent({ eventId: result.eventId });
console.log(event.jobs);
```

### Python

```python
from qhook_client import ApiClient, Configuration
from qhook_client.api import IngestApi, ManagementApi

config = Configuration(host="http://localhost:8888")
config.access_token = "your-token"

with ApiClient(config) as client:
    # Send an event
    ingest = IngestApi(client)
    result = ingest.send_event("app", "order.created", body={"order_id": "ORD-123"})
    print(result.event_id, result.jobs_created)

    # Inspect an event
    mgmt = ManagementApi(client)
    event = mgmt.get_event(result.event_id)
    print(event.jobs)
```

## Regeneration

After updating `docs/openapi.yaml`, regenerate:

```bash
bash sdks/generate.sh all
```

The generated code tracks the OpenAPI spec version — always regenerate after spec changes.
