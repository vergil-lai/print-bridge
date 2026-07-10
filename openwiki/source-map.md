# Source Map

A developer's guide to the PrintBridge codebase: file responsibilities, what to watch out for when modifying each area, and key design decisions.

## Project Layout

```
PrintBridge/
├── src-tauri/          # Rust backend (Tauri 2 + Axum)
│   ├── src/
│   │   ├── main.rs         # Binary entry, CLI/GUI dispatch
│   │   ├── lib.rs          # Tauri app setup, AppState wiring
│   │   ├── runtime.rs      # Headless serve runtime
│   │   ├── server.rs       # Axum HTTP/WebSocket server
│   │   ├── protocol.rs     # WebSocket message types + validation
│   │   ├── queue.rs        # Serial print queue + job pipeline
│   │   ├── printing/       # Platform print backends
│   │   ├── document.rs     # Format detection, image→PDF
│   │   ├── office.rs       # Office detection + conversion
│   │   ├── download.rs     # File download to temp
│   │   ├── config.rs       # Config structs, load/save, paths
│   │   ├── config_transfer.rs  # Encrypted config import/export
│   │   ├── ip_whitelist.rs # IP/CIDR validation + runtime checks
│   │   ├── cli.rs          # CLI command structure + handlers
│   │   ├── service_manager.rs  # systemd/launchd install
│   │   ├── agent_guard.rs  # Instance exclusivity (port probe)
│   │   ├── remote_*.rs     # Remote task polling subsystem
│   │   ├── task_history.rs # SQLite task history store
│   │   ├── app_state.rs    # Shared AppState container
│   │   ├── tray.rs         # System tray
│   │   ├── commands.rs     # Tauri commands
│   │   ├── logs.rs         # In-memory log ring buffer
│   │   └── test_print.rs   # Calibration test page
│   ├── resources/
│   │   └── windows/SumatraPDF.exe
│   ├── capabilities/       # Tauri permission capabilities
│   ├── tests/              # Rust integration tests
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                # Vue 3 frontend
│   ├── main.ts             # App entry
│   ├── App.vue             # Monolithic settings UI (all tabs)
│   ├── api.ts              # Tauri invoke + HTTP fetch wrappers
│   ├── types.ts            # Shared TypeScript types
│   ├── i18n.ts             # vue-i18n setup (zh-CN, en)
│   └── updater.ts          # Tauri updater wrapper
├── examples/           # Server-side reference implementations
│   ├── remote-task/        # HTTP poll/report server (Node/PHP/Go)
│   └── config-transfer/    # Encrypted config generators (Node/PHP/Go)
├── scripts/            # Build/release tooling
├── docs/               # Existing technical documentation
├── tools/              # SumatraPDF version pinning docs
└── .github/workflows/  # CI (release + OpenWiki)
```

## Rust Backend Files

### Entry Points

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `main.rs` | Binary entry. Checks `argv[1]` for CLI commands; dispatches to CLI or `lib::run()`. | The CLI/GUI dispatch is in `is_cli_invocation()` — adding new CLI subcommands requires updating the detection list. |
| `lib.rs` | Tauri app setup: config load, agent guard, tray setup, print backend resolution, AppState construction, async task spawning, window close intercept. | All three async tasks (server, queue worker, remote worker) are spawned here. Window close is intercepted to hide-to-tray. |
| `runtime.rs` | Headless `serve` runtime. Same three-task setup as GUI but without Tauri. | Checks agent guard before starting; exits cleanly (0) if another instance is running. |

### Server & Protocol

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `server.rs` (~46 KB) | Axum HTTP routes, WebSocket handler, IP whitelist middleware, CORS. Binds `0.0.0.0:{port}`. | CORS is hardcoded to settings UI origins only. `ws_handler` validates Origin at upgrade time. All routes pass through IP middleware. |
| `protocol.rs` (~14 KB) | `ClientMessage`/`ServerMessage` types, `JobStatus` enum, `ErrorCode` enum, validation logic, per-connection status filtering. | `validate_for_acceptance` is the gatekeeper for job acceptance. Error codes are protocol-stable — adding/changing them affects SDK and consumers. |

### Print Queue & Pipeline

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `queue.rs` (~45 KB) | `QueueState` (FIFO + dedup), `run_worker` (serial loop), `process_job_inner` (download → convert → print), `accept_job`/`accept_batch`. | The largest file. Single worker = strict serial execution. `prepare_printable_pdf` handles format detection + conversion. Temp file cleanup is critical. |
| `printing/mod.rs` | `PrintBackend` trait, platform dispatch (`default_backend`), `sumatra_print_settings`. | Backend is compile-time selected (`#[cfg(target_os = ...)]`). Changing print settings affects all platforms. |
| `printing/cups.rs` | macOS/Linux backend: `lp`, `lpstat`, `lpoptions`. Job tracking via `lpstat -W completed`. | Parsing `lp` output for job ID is fragile — changes in CUPS output format could break it. |
| `printing/windows.rs` | Windows backend: SumatraPDF CLI for PDF, Win32 Spooler API for raw. | No job tracking (`tracking_supported: false`). SumatraPDF path resolved from Tauri resources. |
| `document.rs` | Magic byte detection (PDF/PNG/JPEG), image→PDF conversion via `printpdf`. | 203 DPI assumption for label printers. Image is fit-contained to paper dimensions. |
| `office.rs` | Office format detection (docx/xlsx/pptx via ZIP internals), `office_to_pdf` via `office2pdf`. | Conversion fidelity depends on `office2pdf` crate — not guaranteed to match MS Office/WPS. |
| `download.rs` | `download_to_temp`: HTTP/HTTPS streaming + data URL. Two-layer size enforcement. Timeout. | Content-Length check + streaming byte count. Partial file cleanup on error. |

### Config & Security

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `config.rs` | `AgentConfig` + section structs, defaults, `load`/`save`, `normalized`, data dir resolution. | `normalized()` forces `host=127.0.0.1` and normalizes IPs. Config fields affect GUI, CLI, remote worker, import/export, and frontend types — all must be checked together. |
| `config_transfer.rs` (~33 KB) | Encrypted export/import: Argon2id + AES-256-GCM, field selection, merge, preview diff. | Bearer token has special protection (empty/null → preserve current). `double_option` serde module handles `Option<Option<String>>`. |
| `ip_whitelist.rs` | IP/CIDR validation, `is_client_ip_allowed` runtime check, rejects allow-all entries. | `127.0.0.1` is forced and immutable. Uses `ipnet` for CIDR — no hand-written bit operations. Does NOT trust proxy headers. |

### CLI & Services

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `cli.rs` (~37 KB) | Clap command structure, all CLI handlers (printer, paper, origin, remote, task, serve). | `serve install/uninstall` is `#[cfg(not(target_os = "windows"))]`. Remote URL must be http/https. |
| `service_manager.rs` | systemd user service + launchd LaunchAgent install/uninstall. | Platform-specific: Linux creates `~/.config/systemd/user/`, macOS creates `~/Library/LaunchAgents/`. Windows returns `UnsupportedPlatform`. |
| `agent_guard.rs` | TCP port probe to detect running instance. Sends `/health` to distinguish PrintBridge from other services. | 300ms timeout. GUI aborts startup; headless exits cleanly. |

### Remote Polling

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `remote_client.rs` | HTTP client: `fetch_tasks`, `report_status`, `test_connection`. Common headers (Bearer, device ID/name). | `is_configuration_status()` treats 401/403/404 as config errors → backoff, not retry. |
| `remote_protocol.rs` | `RemoteTask` enum (print / print_batch), `parse_remote_tasks` (flexible parser). | Parser accepts single object, array, null, or empty string. |
| `remote_store.rs` | SQLite: `remote_jobs` (dedup) + `remote_status_events` (outbox with exponential backoff). | `INSERT OR IGNORE` for idempotency. Unique index on `(job_id, status)` prevents duplicate reports. |
| `remote_worker.rs` | Infinite loop: poll → enqueue → report. Config error pauses. | Wakes on `remote_notify` (config changes). Report backoff via `retry_at_string`. |

### History & State

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `task_history.rs` | SQLite: `task_history_jobs` (aggregate) + `task_history_events` (append-only). `record_event` UPSERT with COALESCE. | `finished_at` set only for terminal statuses. `clear()` deletes both tables in one transaction. |
| `app_state.rs` | `AppState` — shared container for config, queue, logs, status events, print backend, stores. | The `status_events` broadcast channel has capacity 128 — slow consumers may miss events. |
| `logs.rs` | `LogStore` — in-memory ring buffer (500 entries). Not persisted. | Drops oldest when full. |
| `commands.rs` | Tauri commands: config CRUD, export/import, test connection, logs, task history, print test. | `print_test` is settings UI origin only. |
| `tray.rs` | System tray: Open Settings, Test Print, View Logs, Restart, Launch at Startup, Quit. Localized (zh-CN/en). | Left-click shows main window. `toggle_autostart` persists to config. |
| `test_print.rs` | Calibration test page generation. | Uses default printer + paper. |

## Frontend Files

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `App.vue` (~71 KB) | Monolithic SFC with all UI: Settings, Remote, Website whitelist, IP whitelist, Tasks, About tabs. No router. | Single file contains all UI logic. Uses shadcn-vue Tabs for navigation. Config changes that change port trigger app relaunch in production. |
| `api.ts` | Two channels: `invoke()` for Tauri commands, `fetch()` for local HTTP (printers, papers). | `fetchPrinters`/`fetchPapers` hit `http://127.0.0.1:{port}/printers` directly, not Tauri invoke. |
| `types.ts` | Shared TypeScript types: `AgentConfig`, `PrinterInfo`, `PaperInfo`, `TaskHistoryJob`, `TaskHistoryEvent`. | Must match Rust structs — changes require updating both sides. |
| `i18n.ts` | vue-i18n v11 (composition mode). zh-CN (default) + en. ~140 keys. | Task status/source labels are inline lookup tables in App.vue, not in i18n messages. |
| `updater.ts` | Tauri updater wrapper: check, download+install, relaunch. | Progress via `DownloadEvent` callbacks. |
| `main.ts` | App entry: creates Vue app, installs vue-i18n, mounts. Minimal. | |
| `vite.config.ts` | Vue + Tailwind v4. Dev port 1420 (strict). Ignores `src-tauri/`. | `@` alias → `./src`. |

## Supporting Files

| Path | Purpose |
|------|---------|
| `src-tauri/tauri.conf.json` | App config: identifier `com.vergil.printbridge`, window 960×680, updater endpoint, bundle settings |
| `src-tauri/tauri.windows.conf.json` | Windows override: bundles `SumatraPDF.exe` as resource |
| `src-tauri/capabilities/default.json` | Main window permissions: core, dialog, opener defaults |
| `src-tauri/capabilities/desktop.json` | Desktop permissions: autostart, log, updater, process, dialog, opener |
| `scripts/release.mjs` | Interactive release: version sync (3 files), tag check, push to `release` branch |
| `scripts/patch-updater-json.mjs` | Post-release: rewrites `latest.json` asset URLs from API to download URLs |
| `scripts/verify-config-transfer-examples.mjs` | Runs all config-transfer example self-tests |
| `.github/workflows/release.yml` | CI: macOS (ARM+Intel), Linux, Windows builds via `tauri-action` |
| `.github/workflows/openwiki-update.yml` | Daily OpenWiki doc refresh |
| `tools/sumatra/README.md` | SumatraPDF 3.6.1 64-bit version pinning + SHA-256 hashes |
| `AGENTS.md` | AI agent development instructions (Chinese) |
| `docs/printbridge-technical.md` | Detailed technical documentation (Chinese) |
| `docs/printbridge-technical_en.md` | English technical documentation |
