---
layout: default
title: Workflows
---

# Workflows

qhook supports event-driven workflows — sequential multi-step pipelines defined in YAML. Each step makes an HTTP call and chains its response to the next step. Think of it as a lightweight alternative to AWS Step Functions.

## Basic workflow

```yaml
workflows:
  order-pipeline:
    source: app
    events: [order.created]
    steps:
      - name: validate
        url: http://backend:3000/validate
      - name: fulfill
        url: http://backend:3000/fulfill
      - name: notify
        url: http://backend:3000/notify
```

When an `order.created` event arrives, qhook executes the steps sequentially. Each step's HTTP response body becomes the next step's request body.

## Data flow

Each step has three optional data flow controls:

| Field | Purpose | Example |
|-------|---------|---------|
| `input` | Transform payload before HTTP call | `'{"user_id": "{{$.id}}"}'` |
| `result_path` | Merge HTTP response into payload | `"$.enrichment"` |
| `output` | Transform payload after merge | `'{"summary": "{{$.result}}"}'` |

### input

Reshapes the payload before sending to the step's URL. Uses `{{$.path}}` template syntax (same as handler `transform`).

```yaml
steps:
  - name: enrich
    url: http://backend:3000/enrich
    input: '{"user_id": "{{$.id}}"}'
```

### result_path

Controls how the step's HTTP response is merged into the running payload:

| Value | Behavior |
|-------|----------|
| _(not set)_ or `"$"` | Response replaces the entire payload |
| `"$.field"` | Response is nested under `field` in the payload |
| `"null"` | Response is discarded; payload unchanged |

```yaml
steps:
  - name: enrich
    url: http://backend:3000/enrich
    result_path: "$.enrichment"   # response merged as { ...payload, "enrichment": response }
  - name: process
    url: http://backend:3000/process
    # receives: { ...original, "enrichment": { ...enrich response } }
```

## Retry

Each step can override the default retry policy. Use `errors` to limit retries to specific error types.

```yaml
steps:
  - name: validate
    url: http://backend:3000/validate
    retry:
      max: 3
      errors: [5xx, timeout]  # only retry on server errors and timeouts
```

Error types: `timeout`, `5xx`, `4xx`, `network`, `all` (default).

## Catch

After retries are exhausted, `catch` routes to a named fallback step based on the error type.

```yaml
steps:
  - name: validate
    url: http://backend:3000/validate
    retry:
      max: 3
      errors: [5xx, timeout]
    catch:
      - errors: [4xx]
        goto: handle-bad-request
      - errors: [all]
        goto: alert-ops
  - name: fulfill
    url: http://backend:3000/fulfill
  - name: handle-bad-request
    url: http://backend:3000/bad-request
    end: true
  - name: alert-ops
    url: http://backend:3000/alert
    end: true
```

Catch blocks are evaluated in order — the first matching error type wins.

## on_failure

By default, a step failure stops the workflow (`on_failure: stop`). Set `on_failure: continue` to proceed to the next step with error information.

```yaml
steps:
  - name: optional-enrichment
    url: http://backend:3000/enrich
    on_failure: continue
  - name: process
    url: http://backend:3000/process
    # receives: { "error": "...", "failed_step": "optional-enrichment", ...original }
```

## end

Mark a step with `end: true` to terminate the workflow after that step completes, skipping any remaining steps.

```yaml
steps:
  - name: check
    url: http://backend:3000/check
    end: true     # workflow completes here
  - name: never-runs
    url: http://backend:3000/next
```

## Choice step

A choice step evaluates conditions against the payload and routes to a named step. No HTTP call is made — it's pure routing logic.

```yaml
steps:
  - name: route
    type: choice
    choices:
      - when: "$.amount >= 10000"
        goto: high-value
      - when: "$.category == premium"
        goto: premium
    default: standard
  - name: high-value
    url: http://backend:3000/high-value
    end: true
  - name: premium
    url: http://backend:3000/premium
    end: true
  - name: standard
    url: http://backend:3000/standard
    end: true
```

Conditions use the same syntax as handler `filter` expressions: `==`, `!=`, `>=`, `>`, `<=`, `<`, `in [...]`, and truthy checks. Evaluated in order — the first match wins.

## Parallel step

A parallel step executes multiple branches concurrently. After all branches complete, results are merged as an object keyed by branch name and passed to the next step.

```yaml
steps:
  - name: checks
    type: parallel
    branches:
      - name: credit
        url: http://credit-service/check
      - name: fraud
        url: http://fraud-service/check
    result_path: "$.checks"
  - name: process
    url: http://backend:3000/process
    # receives: { ...original, "checks": { "credit": {...}, "fraud": {...} } }
```

Each branch makes an independent HTTP call. All branches receive the same input payload.

## Map step

A map step iterates over an array in the payload, calling the same URL for each item. Results are collected as an array.

```yaml
steps:
  - name: process-items
    type: map
    items_path: "$.items"
    url: http://backend:3000/process-item
    result_path: "$.results"
  - name: summarize
    url: http://backend:3000/summarize
    # receives: { ...original, "results": [{...}, {...}, ...] }
```

Each item is sent as the full request body to the URL. Items are processed concurrently.

## Coexistence with handlers

Workflows and handlers can coexist. The same event can trigger both a handler and a workflow:

```yaml
handlers:
  log-event:
    source: app
    events: [order.created]
    url: http://logger:3000/log

workflows:
  order-pipeline:
    source: app
    events: [order.created]
    steps:
      - name: process
        url: http://backend:3000/process
```

## Wait step

A wait step pauses the workflow for a specified duration before proceeding. No HTTP call is made — the next step's job is simply scheduled with a future `scheduled_at`.

### Fixed delay

```yaml
steps:
  - name: delay
    type: wait
    seconds: 60          # wait 60 seconds before next step
  - name: send-reminder
    url: http://backend:3000/remind
```

### Dynamic timestamp

Use `timestamp_path` to read a future timestamp from the payload. The workflow resumes at that time.

```yaml
steps:
  - name: schedule
    type: wait
    timestamp_path: "$.scheduled_at"   # e.g. "2026-03-10T15:00:00Z"
  - name: execute
    url: http://backend:3000/execute
```

Either `seconds` or `timestamp_path` must be set (not both). If `timestamp_path` resolves to a past time, the next step runs immediately.

## Callback step

A callback step pauses the workflow and waits for an external system to resume it via an HTTP call. This is useful for human approval, third-party processing, or any asynchronous operation.

```yaml
steps:
  - name: request-approval
    url: http://backend:3000/request-approval
  - name: wait-for-approval
    type: callback
    callback_timeout: 3600   # optional: expire after 1 hour (seconds)
  - name: fulfill
    url: http://backend:3000/fulfill
```

When a callback step is reached, qhook creates a waiting job with a unique token and returns it in the workflow run. The external system calls:

```
POST /callback/{token}
Content-Type: application/json

{"approved": true, "reviewer": "alice"}
```

- **200 OK**: The callback was accepted. The JSON body becomes the next step's input payload.
- **404 Not Found**: The token is invalid or has already been consumed/expired.

If `callback_timeout` is set and no callback is received within that time, the step is marked as failed and the workflow proceeds according to the step's `on_failure` setting (default: `stop`).

## Sub-workflow step

A workflow step invokes another workflow as a sub-workflow. The current workflow pauses until the sub-workflow completes, then continues to the next step. This enables reuse of common step sequences.

```yaml
workflows:
  notify-all:
    source: app
    steps:
      - name: slack
        url: http://slack-bot:3000/notify
      - name: email
        url: http://email-service:3000/send

  order-pipeline:
    source: app
    events: [order.created]
    steps:
      - name: fulfill
        url: http://backend:3000/fulfill
      - name: notify
        type: workflow
        workflow: notify-all
```

When the `notify` step is reached, qhook creates a new workflow run for `notify-all` and executes its steps. After `notify-all` completes, `order-pipeline` advances to its next step (or finishes if `notify` was the last step).

- The sub-workflow receives the current step's payload as input
- Self-referencing (recursive) workflows are rejected at config validation
- Sub-workflows can be nested (a sub-workflow can invoke another sub-workflow)

## Workflow timeout

Set `timeout` on a workflow to limit total execution time. If the workflow runs longer than the specified seconds, subsequent steps are skipped and the workflow is marked as failed.

```yaml
workflows:
  order-pipeline:
    source: app
    events: [order.created]
    timeout: 300           # workflow must complete within 5 minutes
    steps:
      - name: validate
        url: http://backend:3000/validate
      - name: fulfill
        url: http://backend:3000/fulfill
```

This is especially useful for workflows with wait or callback steps to prevent them from running indefinitely.

## CLI

```bash
# List workflow runs
qhook workflow-runs list
qhook workflow-runs list --status completed

# Redrive a failed workflow (restart from the failed step)
qhook workflow-runs redrive <RUN_ID>
```

## Full step reference

```yaml
# Workflow-level config
workflows:
  my-workflow:
    source: app
    events: [order.created]
    timeout: 300               # overall workflow timeout in seconds (optional)
    steps:
      # Task step (HTTP call)
      - name: step-name          # required, unique within workflow
        url: http://...           # HTTP endpoint to call
        type: http                # http (default), grpc, choice, parallel, map, wait, callback, workflow
        timeout: 30               # per-step timeout in seconds
        input: '...'              # input transform template
        result_path: "$.field"    # response merge path
        output: '...'             # output transform template
        retry:
          max: 5                  # max retry attempts
          errors: [5xx, timeout]  # error types to retry
        catch:
          - errors: [4xx]         # error types to catch
            goto: fallback-step   # step name to jump to
        on_failure: stop          # stop (default) or continue
        end: false                # true = terminate workflow after this step

      # Choice step (conditional routing)
      - name: route
        type: choice
        choices:
          - when: "$.field >= 100" # filter condition
            goto: step-name        # target step
        default: fallback-step     # if no condition matches

      # Parallel step (concurrent branches)
      - name: checks
        type: parallel
        branches:
          - name: branch-a        # unique branch name
            url: http://...        # HTTP endpoint
        result_path: "$.results"   # merged results as {branch-a: {...}, ...}

      # Map step (iterate over array)
      - name: process
        type: map
        items_path: "$.items"      # JSONPath to array
        url: http://...            # called per item
        max_concurrency: 10        # optional limit
        result_path: "$.results"   # collected results as [...]

      # Wait step (delay before next step)
      - name: delay
        type: wait
        seconds: 60               # fixed delay in seconds
        # OR
        # timestamp_path: "$.scheduled_at"  # dynamic timestamp from payload

      # Callback step (wait for external signal)
      - name: wait-for-signal
        type: callback
        callback_timeout: 3600     # optional: expire after N seconds
        # External system calls POST /callback/{token} with JSON body to resume

      # Sub-workflow step (invoke another workflow)
      - name: notify
        type: workflow
        workflow: notify-all       # name of workflow to invoke
```
