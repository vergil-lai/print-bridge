# Printing Pipeline

The printing pipeline is the core processing path: from job acceptance through download, format conversion, platform-specific printing, and status tracking. All jobs — whether from WebSocket or remote polling — flow through the same serial FIFO queue.

## Serial Queue Architecture

The print queue is a strict FIFO implemented with `VecDeque<QueuedJob>` in `QueueState` (`queue.rs`):

```rust
struct QueueState {
    pending: VecDeque<QueuedJob>,
    seen_job_ids: HashSet<String>,
    seen_batch_ids: HashSet<String>,
}
```

**Dedup:** Every `job_id` is checked against `seen_job_ids` before acceptance. Duplicates are rejected with `JOB_DUPLICATED`. Batch jobs additionally check `batch_id` uniqueness against `seen_batch_ids`.

**Serial execution** is enforced by a single worker loop:

```rust
loop {
    if let Some(job) = state.queue.lock().await.pop_next() {
        process_job(&state, job).await;  // completes fully before next
        continue;
    }
    state.queue_notify.notified().await;  // sleep until notified
}
```

A `print_lock` (`Arc<Mutex<()>>`) provides an additional guarantee: even if multiple workers existed, only one platform print command executes at a time. Currently only one worker runs per process.

## Job Acceptance Flow

Jobs enter the queue from two sources:

1. **WebSocket** — `protocol.rs` validates each `print`/`print_batch` message (`validate_for_acceptance`), then calls `queue.accept_job()` or `queue.accept_batch()`. The connection immediately receives `JobStatus::Queued`.

2. **Remote polling** — `remote_worker.rs` validates remote tasks and calls `queue.accept_remote_job()` or `queue.accept_remote_batch()`, after SQLite-based dedup in `remote_store.rs`.

In both cases, `queue_notify.notify_one()` wakes the worker.

## Job Processing Pipeline (`process_job_inner`)

Once dequeued, each job follows this path:

```
Pop from queue
    │
    ├── Format = html / raw-html?
    │   ├── html: validate file_url (absolute http/https) → render via Chrome/Chromium
    │   └── raw-html: render inline HTML via Chrome/Chromium
    │       └── Browser launches with filtering proxy → CDP PrintToPDF → temp PDF → print
    │
    ├── Format = raw?
    │   ├── Yes → decode base64 → resolve printer → print_raw() → track status
    │
    ├── Download file_url to temp (pdf / image / office)
    │   ├── HTTP/HTTPS: stream with size enforcement (Content-Length + byte count)
    │   └── data: URL: base64-decode directly
    │
    ├── Resolve printer (specified or default) + paper (specified or default)
    │
    ├── Convert to PDF if needed
    │   ├── Office (docx/xlsx/pptx) → office_to_pdf() via LibreOffice or Windows COM
    │   ├── Image (PNG/JPEG) → image_to_pdf() via printpdf crate (fit-contain, 203 DPI)
    │   └── PDF → normalize_pdf_path() (ensure .pdf extension for print tools)
    │
    ├── Submit to OS print queue via platform backend
    │   ├── Windows: SumatraPDF.exe -silent -print-to ...
    │   └── macOS/Linux: lp -d "{printer}" -n {copies} -o media={media}
    │
    ├── Track status (CUPS only)
    │
    └── Cleanup temp files
```

On any `Err` at any stage, the job is logged as `JobStatus::Failed` with the error message.

## Format Detection

PrintBridge uses **magic byte detection** to verify file content matches the declared format:

| Magic Bytes | Format | Detection Location |
|-------------|--------|-------------------|
| `%PDF-` | PDF | `document.rs` |
| `\x89PNG\r\n\x1a\n` | PNG | `document.rs` |
| `\xFF\xD8\xFF` | JPEG | `document.rs` |
| ZIP with `word/document.xml` | Docx | `office.rs` |
| ZIP with `xl/workbook.xml` | Xlsx | `office.rs` |
| ZIP with `ppt/presentation.xml` | Pptx | `office.rs` |

If declared format doesn't match detected bytes → `FORMAT_MISMATCH` error.

## Image → PDF Conversion

Images are converted to single-page PDFs using the `printpdf` crate (`document.rs`, `image_to_pdf`):

- Image is **fit-contained** into the target paper dimensions (centered, aspect-preserved)
- Default DPI assumption: **203 DPI** (standard for label printers)
- Paper dimensions come from the job's `paper` field or config default

## Office → PDF Conversion

Office documents (docx/xlsx/pptx) are converted to PDF via the platform's native Office software (`office.rs` + `office/`). On macOS/Linux, LibreOffice (`soffice`/`libreoffice`) is invoked in an isolated profile with macro security level set to maximum. On Windows, the native Windows COM interface is used. Conversion has a 120-second timeout. Print results depend on LibreOffice rendering — not guaranteed to match Microsoft Office or WPS exactly.

## HTML Rendering Pipeline

HTML and raw-html jobs bypass the download stage. Instead, they are rendered to a temp PDF by the `HtmlRenderer` (default: `BrowserHtmlRenderer`):

1. **Browser discovery** — finds Chrome, Chromium, or Edge in platform-specific locations
2. **Filtering proxy** — a local HTTP proxy intercepts all browser resource requests. `ResourcePolicy` blocks non-public IPs (loopback, private, link-local, multicast, `file:`, `data:` schemes). DNS is resolved before connecting to prevent DNS rebinding.
3. **Render** — the browser navigates to the target URL (or loads inline HTML), waits `wait_ms` milliseconds (default 1000, max 30000), then exports to PDF via CDP `Page.printToPDF`.
4. The resulting temp PDF is submitted through the normal PDF print path, then cleaned up.

If a blocked resource is detected, the render fails with `HtmlRenderError::BlockedResource` and the job is marked as failed.

## Download Safety (`download.rs`)

`download_to_temp` enforces safety at two levels:
1. **Content-Length header check** — rejects before download starts if the header exceeds the limit
2. **Streaming byte enforcement** — also enforces the limit during streaming download

Downloads have a configurable timeout (`limits.download_timeout_seconds`, default 30s). Partial files are cleaned up on error.

Supported URL schemes: `http://`, `https://`, `data:application/pdf;base64,...`.

## Status Tracking

After job submission to the OS print queue:

| Platform | Tracking | How |
|----------|----------|-----|
| macOS/Linux | **Yes** | `lpstat -W completed -o` — checks if the system job ID appears in completed list |
| Windows | **No** | SumatraPDF and Win32 spooler API don't expose trackable status; `tracking_supported: false` |

On macOS/Linux, tracking can result in `Completed`, `Failed`, or `Unknown` (if the job can't be found).

## Dedup and Idempotency

- **WebSocket:** `seen_job_ids` in in-memory `QueueState` — persists for the lifetime of the process
- **Remote polling:** `remote_jobs` table in `remote.sqlite3` — persists across restarts (see [Remote Task Polling](remote-polling.md))

Both use `job_id` as the dedup key.

## Source References

| Area | File |
|------|------|
| Queue state + worker loop + job processing | `crates/runtime/src/queue.rs` |
| Format detection + image→PDF | `crates/runtime/src/document.rs` |
| Office detection + conversion | `crates/runtime/src/office.rs`, `office/libreoffice.rs`, `office/windows.rs` |
| HTML rendering (browser, proxy, policy) | `crates/runtime/src/html/` |
| Download to temp | `crates/runtime/src/download.rs` |
| Platform print backends | `crates/runtime/src/printing/mod.rs`, `cups.rs`, `windows.rs` |
| Message validation + job types | `crates/core/src/protocol.rs` |
