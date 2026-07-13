# Remote Task Polling

Remote task polling lets PrintBridge operate as an unattended agent on workstations, store terminals, or warehouse computers. Instead of a browser pushing jobs via WebSocket, a business server maintains pending print tasks. The local agent periodically polls for tasks, submits them to the OS print queue, and reports execution status back to the server.

This is suited for scenarios where the system creates tasks and expects a specific terminal to print automatically — production labels, shipping labels, picking lists, receipts.

## Polling Protocol

The agent uses the same `remote.endpoint_url` for both operations:

```
GET  {endpoint_url}   → fetch pending tasks
POST {endpoint_url}   → report task status
```

### Authentication Headers

```
Authorization: Bearer <bearer_token>        (if configured)
X-PrintBridge-Device-Id: <device_id>        (if configured)
X-PrintBridge-Device-Name: <device_name>    (if configured)
```

All three fields are optional and independent — only the configured ones are sent.

### Task Fetch Response

The `GET` response is flexible:
- `204 No Content` or empty body → no tasks
- `null` → no tasks
- Single task object → one task
- Array of task objects → multiple tasks

Tasks use the same field structure as WebSocket jobs:

```json
{
  "type": "print",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "format": "pdf",
  "file_url": "https://example.com/label.pdf",
  "copies": 1
}
```

Batch tasks carry `batch_id` and a `jobs` array. `job_id` is the dedup key — tasks with already-seen `job_id` values are silently skipped.

### Status Report Body

```json
{
  "event": "status",
  "event_id": "8c3f0f3a-...",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "status": "success",
  "message": "submitted to system print queue",
  "occurred_at": "2026-07-06T10:00:00Z",
  "device_id": "f77160d2-...",
  "device_name": "packing-station-01"
}
```

`event_id` is a UUID v4 generated locally and persisted in SQLite — the server can use it as an idempotency key.

## Status Mapping

PrintBridge reports only three remote statuses:

| Local Queue Status | Remote Status Reported |
|-------------------|----------------------|
| `queued` | `accepted` |
| `submitted` | `success` |
| `failed` | `failed` |
| `cancelled` | `failed` |

`downloading`, `printing`, `completed`, and `unknown` are **not** reported to the remote server — they remain in local logs and task history only.

## Worker Loop (`remote_worker.rs`)

The remote worker runs an infinite loop:

1. Read config; if `remote.enabled` is false → `await` on `remote_notify` (wakes on config change)
2. **Poll:** `fetch_tasks` → validate → dedup → enqueue to print queue
3. **Report:** deliver pending status events from the SQLite outbox
4. If a configuration error (HTTP 401/403/404) occurred → `await` on `remote_notify` (stops hammering the server)
5. Sleep for `poll_interval_seconds` (default 10, minimum 3)
6. Repeat

## SQLite Persistence (`remote_store.rs`)

Remote state is persisted in `remote.sqlite3` with two tables:

### `remote_jobs` — Dedup Table

| Column | Purpose |
|--------|---------|
| `job_id` (PK) | Dedup key |
| `request_id`, `batch_id` | Original request tracking |
| `status` | Current status |
| `first_seen_at`, `updated_at` | Timestamps |

`record_job_if_new` uses `INSERT OR IGNORE` — returns `true` only on first insert. This provides cross-restart dedup: if the agent restarts, tasks already in the queue are not re-enqueued.

### `remote_status_events` — Status Outbox

| Column | Purpose |
|--------|---------|
| `event_id` (PK) | UUID v4, idempotency key |
| `job_id`, `status`, `message` | Status to report |
| `delivery_state` | `pending` / `delivered` / `abandoned` |
| `retry_count` | Incremented on failure |
| `next_retry_at` | Exponential backoff timestamp |
| `last_error` | Last failure message |

Unique index on `(job_id, status)` prevents duplicate status reports per job.

### Delivery with Exponential Backoff

`report_pending_once` fetches up to 20 pending events and POSTs each:
- **Success** (HTTP 2xx) → `mark_delivered`
- **Failure** (non-2xx or network error) → `mark_delivery_failed` → increments `retry_count`, sets `next_retry_at` with exponential backoff, abandons after `max_report_retries` (default 10)
- **Configuration error** (401/403/404) → propagates immediately, pauses polling and reporting until config is fixed

## Connection Test

The settings UI's "Test Connection" button and remote config save trigger a test via `remote_client::test_connection`:
1. Sends `GET` to the endpoint with `X-PrintBridge-Test: true` header
2. Sends `POST` with the same test header
3. Server should respond `204 No Content` for test requests

## Configuration Error Handling

HTTP 401, 403, and 404 are treated as **configuration errors** — not transient failures. When encountered, the remote worker pauses both polling and status reporting, and waits on `remote_notify` (triggered when the user updates remote config). This prevents hammering a misconfigured or unauthorized endpoint.

## Server Implementation Examples

Reference implementations in `examples/remote-task/` demonstrate the server-side HTTP API:

| Language | File | Example Task |
|----------|------|-------------|
| Node.js | `remote-task-server.mjs` | Single PDF (`JOB-NODE-PDF`) |
| PHP | `remote-task-server.php` | Single image (`JOB-PHP-IMAGE`) |
| Go | `remote-task-server.go` | Batch with PDF + image (`BATCH-GO-SAMPLE`) |

All three:
- Listen on `127.0.0.1:18080`
- Use Bearer token `dev-token`
- Return `204` for `X-PrintBridge-Test: true` requests
- `GET` → return task JSON; `POST` → log status, return `204`

## Source References

| Area | File |
|------|------|
| HTTP client (fetch + report + test) | `crates/runtime/src/remote_client.rs` |
| Task/batch protocol structures | `crates/core/src/remote_protocol.rs` |
| SQLite store (dedup + outbox) | `crates/runtime/src/remote_store.rs` |
| Worker loop (poll + report + backoff) | `crates/runtime/src/remote_worker.rs` |
