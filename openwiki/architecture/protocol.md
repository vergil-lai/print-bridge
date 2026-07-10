# Print Protocol & API

PrintBridge exposes two communication channels: a WebSocket endpoint for real-time browser-initiated printing, and an HTTP REST API for settings UI, diagnostics, and printer discovery.

The WebSocket protocol is the primary integration path for web pages. The HTTP API is primarily for the settings UI and diagnostic tools. Both are served by the same Axum server on `0.0.0.0:{port}` (default `17890`).

## HTTP API

All HTTP routes are subject to IP whitelist middleware. CORS is restricted to settings UI origins only (`localhost:1420`, `tauri://localhost`, `http://tauri.localhost`).

| Route | Method | Purpose |
|-------|--------|---------|
| `/health` | GET | Health check → `{"status":"ok","service":"print-bridge"}` |
| `/printers` | GET | List system printers |
| `/printers/{name}/papers` | GET | List paper sizes for a specific printer |
| `/config` | GET | Read current agent config |
| `/config` | POST | Update agent config |
| `/logs` | GET | Recent task log entries (in-memory ring buffer) |
| `/print/test` | POST | Submit a calibration test page (settings UI Origin only) |
| `/ws` | GET | WebSocket upgrade (see below) |

**`POST /print/test`** uses the current default printer and paper to print a calibration page. Returns `202 Accepted`. Restricted to settings UI origins — external pages cannot trigger test prints.

## WebSocket API

### Connection

```
ws://127.0.0.1:17890/ws
```

During the WebSocket handshake, the server validates the `Origin` header against `config.security.allowed_origins`. If the Origin is not in the allowlist, the upgrade is rejected. The connecting client IP must also pass the IP whitelist check (applied as middleware to all routes including `/ws`).

### Client → Server Messages

All messages are JSON with a `type` discriminator field (`#[serde(tag = "type")]`).

| Message Type | Fields | Purpose |
|-------------|--------|---------|
| `ping` | `time` | Heartbeat; server responds with `pong` |
| `get_printers_list` | — | Request list of available printers |
| `get_printer_info` | `printer_name` | Request details for a specific printer |
| `get_print_queue` | — | Request current print queue status |
| `print` | `request_id`, `job_id`, `format`, `file_url`/`data_base64`, `printer_name?`, `copies?`, `paper?` | Submit a single print job |
| `print_batch` | `request_id`, `batch_id`, `jobs[]` | Submit multiple jobs atomically |

### Server → Client Messages

| Message Type | Purpose |
|-------------|---------|
| `pong` | Heartbeat response with `time` |
| `printers_list` | Response to `get_printers_list` |
| `printer_info` | Response to `get_printer_info` |
| `print_queue` | Response to `get_print_queue` |
| `job_status` | Status update for a job (pushed asynchronously) |
| `error` | Error response with `code` and `message` |

### Job Lifecycle

Each WebSocket connection only receives `job_status` events for jobs it submitted (tracked via per-connection `accepted_job_ids: HashSet`).

```
Queued → Downloading → Printing → Submitted → Completed
                                          ↘ Failed
                                          ↘ Unknown
                                          ↘ Cancelled
```

| Status | Meaning |
|--------|---------|
| `queued` | Job accepted into the serial queue |
| `downloading` | Downloading file from `file_url` |
| `printing` | Converting (if needed) and submitting to OS print queue |
| `submitted` | Job accepted by OS print queue (terminal for status reporting) |
| `completed` | OS reports job completed (CUPS only; Windows cannot track) |
| `failed` | Job failed at any stage |
| `unknown` | OS cannot determine final status |
| `cancelled` | Job cancelled |

### Single Print Job Example

```json
{
  "type": "print",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "format": "pdf",
  "printer_name": "Office Printer",
  "file_url": "https://example.com/label.pdf",
  "copies": 1,
  "paper": { "width_mm": 60, "height_mm": 40 }
}
```

- `printer_name` optional — falls back to default printer
- `paper` optional — falls back to default paper
- `copies` optional — defaults to 1; must be ≤ `limits.max_copies`

### Raw Print Job Example

```json
{
  "type": "print",
  "request_id": "REQ-RAW-001",
  "job_id": "JOB-RAW-001",
  "format": "raw",
  "printer_name": "TSC TE244",
  "data_base64": "XlhB..."
}
```

Raw jobs **do not** support `file_url`, `paper`, or `copies`. The `data_base64` bytes are submitted to the OS print queue as-is. PrintBridge does not parse or generate device commands (ESC/POS, TSPL, ZPL, EPL, PCL, PostScript).

### Batch Print Job Example

```json
{
  "type": "print_batch",
  "request_id": "REQ-002",
  "batch_id": "BATCH-001",
  "jobs": [
    { "job_id": "A-001", "format": "image", "file_url": "https://example.com/a.png", "copies": 1 },
    { "job_id": "B-001", "format": "raw", "printer_name": "TSC TE244", "data_base64": "XlhB..." }
  ]
}
```

Batch jobs can mix PDF, image, Office, and raw formats. `batch_id` and all `job_id`s must be unique. Batch size is limited by `limits.max_batch_jobs` (default 20). Batch execution still uses the same serial queue — it is not concurrent printing.

### Job Status Push

```json
{
  "type": "job_status",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "status": "queued",
  "message": "queued"
}
```

### Supported Formats

| Format | Input | Conversion |
|--------|-------|------------|
| `pdf` | `file_url` or `data:application/pdf;base64,...` | None |
| `image` | `file_url` | Image → PDF (fit-contain to paper size, 203 DPI) |
| `docx` / `xlsx` / `pptx` | `file_url` (HTTP/HTTPS only) | Office → PDF via `office2pdf` |
| `raw` | `data_base64` | None — bytes submitted as-is |

Office tasks only support HTTP(S) `file_url`, not data URLs.

### Error Codes

| Code | Meaning |
|------|---------|
| `ORIGIN_NOT_ALLOWED` | WebSocket Origin not in allowlist |
| `INVALID_MESSAGE` | Malformed message |
| `PRINTER_NOT_CONFIGURED` | No default printer set and none specified |
| `PRINTER_NOT_FOUND` | Specified printer does not exist |
| `PAPER_NOT_CONFIGURED` | No default paper set and none specified |
| `PAPER_NOT_FOUND` | Paper size not available for printer |
| `DOWNLOAD_FAILED` | File download failed |
| `FILE_TOO_LARGE` | File exceeds size limit |
| `UNSUPPORTED_FORMAT` | Format not supported |
| `FORMAT_MISMATCH` | Declared format doesn't match file content |
| `OFFICE_CONVERT_FAILED` | Office → PDF conversion failed |
| `PRINT_FAILED` | Print submission to OS failed |
| `JOB_DUPLICATED` | `job_id` already seen |
| `BATCH_DUPLICATED` | `batch_id` already seen |
| `BATCH_TOO_LARGE` | Batch exceeds max jobs limit |
| `COPIES_OUT_OF_RANGE` | Copies exceeds max copies |
| `SERVICE_PORT_IN_USE` | Port already occupied |
| `INTERNAL_ERROR` | Unexpected internal error |

## Source References

| Area | File |
|------|------|
| HTTP routes + WS handler | `src-tauri/src/server.rs` |
| Message types + validation + error codes | `src-tauri/src/protocol.rs` |
| Per-connection status filtering | `src-tauri/src/protocol.rs` (`status_message_for_connection`) |
| Batch acceptance + dedup | `src-tauri/src/queue.rs` (`accept_batch`) |

Detailed protocol examples are in `docs/printbridge-technical.md` (WebSocket API section). Browser integration should use the [print-bridge-sdk](https://github.com/vergil-lai/print-bridge-jssdk) which wraps this protocol.
