# Source Map

A developer's guide to the PrintBridge codebase: file responsibilities, what to watch out for when modifying each area, and key design decisions.

## Project Layout

```
PrintBridge/                       # Cargo workspace (5 members)
├── crates/
│   ├── core/                      # Domain models, protocol, config — framework-free
│   │   └── src/
│   │       ├── config.rs          # AgentConfig + sections, load/save, defaults
│   │       ├── protocol.rs        # ClientMessage/ServerMessage, validation, ErrorCode
│   │       ├── ip_whitelist.rs    # IP/CIDR validation + runtime checks
│   │       ├── printing.rs        # PrintBackend trait, PrinterInfo/PaperInfo types
│   │       ├── queue.rs           # QueueState types, QueueError
│   │       ├── remote_protocol.rs # RemoteTask types + parser
│   │       └── activity.rs        # TaskHistoryJob, TaskHistoryEvent, TaskLogEntry
│   ├── runtime/                   # AgentRuntime, platform adapters, background tasks
│   │   └── src/
│   │       ├── agent.rs           # AgentRuntime + AgentHandle lifecycle
│   │       ├── builder.rs         # RuntimeBuilder (paths + adapters → AgentRuntime)
│   │       ├── state.rs           # AgentState (shared config, queue, stores, adapters)
│   │       ├── server.rs          # Axum router (only /ws exposed)
│   │       ├── queue.rs           # Serial print queue + job processing pipeline
│   │       ├── command_executor.rs # RuntimeCommandExecutor (offline Command executor)
│   │       ├── doctor.rs          # Doctor diagnostic checks
│   │       ├── html/              # HTML → PDF rendering (Chrome/Chromium via CDP)
│   │       ├── printing/          # Platform print backends (CUPS, Windows)
│   │       ├── office/            # Office → PDF conversion (LibreOffice / Windows COM)
│   │       ├── document.rs        # Magic byte detection, image→PDF
│   │       ├── download.rs        # HTTP/HTTPS/data URL download to temp
│   │       ├── ipc/               # Local IPC (Unix socket / Windows named pipe)
│   │       ├── remote_*.rs        # Remote task polling subsystem
│   │       ├── task_history.rs    # SQLite task history store
│   │       ├── logs.rs            # In-memory log ring buffer
│   │       ├── test_print.rs      # Calibration test page
│   │       └── agent_guard.rs     # Instance exclusivity (port probe)
│   └── cli/                       # Command enum, CommandService, CLI parser, IPC client
│       └── src/
│           ├── command.rs         # Command enum + policy (online/offline preference)
│           ├── service.rs         # CommandService (online/offline dispatch)
│           ├── parser.rs          # Clap command structure + CLI handlers
│           ├── client.rs          # LocalClientExecutor (IPC client)
│           ├── output.rs          # CommandResult, CommandError, DoctorReport types
│           ├── policy.rs          # CommandPolicy enum
│           ├── product.rs         # ProductCommandAdapter (autostart, language)
│           ├── interaction.rs     # Terminal prompts
│           └── config_transfer.rs # Encrypted config import/export
├── apps/
│   ├── desktop/                   # Vue 3 frontend + Tauri backend
│   │   ├── src/                   # Frontend (Vue 3 + TypeScript)
│   │   │   ├── App.vue            # Monolithic settings UI
│   │   │   ├── api.ts             # Tauri invoke + HTTP wrappers
│   │   │   ├── types.ts           # Shared TypeScript types
│   │   │   ├── i18n.ts            # vue-i18n (zh-CN, en)
│   │   │   └── updater.ts         # Tauri updater wrapper
│   │   └── src-tauri/             # Desktop Rust backend
│   │       ├── src/
│   │       │   ├── main.rs        # Binary entry
│   │       │   ├── lib.rs         # Tauri app setup + runtime wiring
│   │       │   ├── cli.rs         # Desktop CLI dispatch
│   │       │   ├── product_cli.rs # Desktop product adapter (autostart, language)
│   │       │   ├── commands.rs    # Tauri commands (config, logs, test print)
│   │       │   └── tray.rs        # System tray
│   │       ├── resources/         # SumatraPDF.exe (Windows)
│   │       └── tests/             # Integration tests
│   └── server/                    # Linux headless binary
│       ├── src/
│       │   ├── main.rs            # Binary entry: serve / shared CLI dispatch
│       │   ├── dependencies.rs    # Preflight checks (CUPS, LibreOffice, Chrome)
│       │   ├── parser.rs          # Server CLI args
│       │   ├── paths.rs           # System paths (/etc, /var/lib, /run)
│       │   ├── readiness.rs       # systemd READY/STOPPING notifications
│       │   └── signals.rs         # SIGTERM/SIGINT handling
│       ├── packaging/             # deb/rpm/systemd unit files
│       └── tests/
├── examples/                      # Server-side reference implementations
├── scripts/                       # Build/release tooling
├── docs/                          # Technical documentation
└── .github/workflows/             # CI (release + OpenWiki)
```

## Core Crate (`crates/core`)

Framework-free domain models and protocol types. No dependency on Tauri, Axum, Clap, or SQLite.

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `protocol.rs` | `ClientMessage`/`ServerMessage` types, `JobStatus`, `ErrorCode`, `PrintJobInput` validation, `SupportedFormat` enum. | `validate_for_acceptance` is the gatekeeper for job acceptance. Error codes are protocol-stable. Now includes `Html` and `RawHtml` formats + `html`/`wait_ms` fields. |
| `config.rs` | `AgentConfig` + section structs, defaults, `load`/`save`, `normalized`, data dir resolution. | `normalized()` forces `host=127.0.0.1`. Config types must match frontend `types.ts`. |
| `ip_whitelist.rs` | IP/CIDR validation, `is_client_ip_allowed`, rejects allow-all entries. | `127.0.0.1` is forced and immutable. Does NOT trust proxy headers. |
| `printing.rs` | `PrintBackend` trait, `PrinterInfo`, `PaperInfo`, `PrintOptions`, `PrintSubmission`, `PrintTrackingOutcome`. | These are framework-free types — platform implementations live in `crates/runtime`. |
| `queue.rs` | Queue state types and `QueueError`. | |
| `remote_protocol.rs` | `RemoteTask` enum + flexible parser. | Parser accepts single object, array, null, or empty string. |
| `activity.rs` | `TaskHistoryJob`, `TaskHistoryEvent`, `TaskLogEntry` — shared activity types. | |

## Runtime Crate (`crates/runtime`)

AgentRuntime lifecycle, platform adapters, and all background tasks.

### Agent Lifecycle

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `agent.rs` | `AgentRuntime` (pre-start), `AgentHandle` (running). `start()` binds listener, spawns server/queue/remote/IPC tasks. | All four tasks share a `CancellationToken`. `shutdown()` waits for completion and deletes the IPC socket. |
| `builder.rs` | `RuntimeBuilder` — paths + optional print backend and HTML renderer → `AgentRuntime`. `RuntimePaths` (config, data, runtime dirs). | Defaults: `printing::default_backend()` and `BrowserHtmlRenderer`. |
| `state.rs` | `AgentState` — shared container: config, queue, stores, print backend, HTML renderer, IPC executor. | Cloned cheaply via `Arc`. The `status_events` broadcast has capacity 128. |
| `command_executor.rs` | `RuntimeCommandExecutor` — offline executor for `Command` (used when Agent is not running). | |
| `doctor.rs` | `run_doctor` — read-only diagnostic checks (config validity, data dir, port, printers, browser, office, systemd, remote). | Checks browser (Chrome/Chromium) and LibreOffice availability. |

### Server & IPC

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `server.rs` | Axum router exposing only `/ws`. IP whitelist middleware, WebSocket handler, Origin validation. | No HTTP REST endpoints — config/logs/printers/test-print are handled via Tauri commands or CLI/IPC. |
| `ipc/mod.rs` | Platform dispatch for local IPC (Unix socket on Unix, named pipe on Windows). | 4-byte big-endian length prefix, JSON envelope, protocol v1, max 8 MiB frames. Socket at `agent.sock` with `0660` permissions. |
| `ipc/unix.rs` | Unix domain socket implementation. | |
| `ipc/windows.rs` | Named pipe implementation. | |

### Print Queue & Pipeline

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `queue.rs` | `QueueState` (FIFO + dedup), `run_worker` (serial loop), `process_job_inner` (download → convert → print). Separate paths for `html`, `raw-html`, and `raw` jobs. | Single worker = strict serial execution. `print_html_job` renders to temp PDF then reuses PDF submission path. Temp file cleanup is critical. |
| `printing/mod.rs` | Platform dispatch (`default_backend`), `sumatra_print_settings`. | Backend is compile-time selected (`#[cfg(target_os = ...)]`). |
| `printing/cups.rs` | macOS/Linux backend: `lp`, `lpstat`, `lpoptions`. Job tracking via `lpstat -W completed`. | Parsing `lp` output for job ID is fragile. |
| `printing/windows.rs` | Windows backend: SumatraPDF CLI for PDF, Win32 Spooler API for raw. | No job tracking (`tracking_supported: false`). |
| `document.rs` | Magic byte detection (PDF/PNG/JPEG), image→PDF conversion via `printpdf`. | 203 DPI assumption for label printers. |
| `office.rs` + `office/` | Office format detection + conversion. macOS/Linux: LibreOffice (`libreoffice.rs`). Windows: native COM (`windows.rs`). | Conversion fidelity depends on LibreOffice rendering — not guaranteed to match MS Office. 120s conversion timeout. |
| `download.rs` | `download_to_temp`: HTTP/HTTPS streaming + data URL. Two-layer size enforcement. Timeout. | Content-Length check + streaming byte count. Partial file cleanup on error. |

### HTML Rendering (`html/`)

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `mod.rs` | `HtmlRenderer` trait, `HtmlRenderRequest`, `HtmlSource` (Url or Inline), `HtmlRenderError`. | Trait is async (`Pin<Box<dyn Future>>`), allows test injection. |
| `browser.rs` | `BrowserHtmlRenderer` — finds Chrome/Chromium/Edge, launches with proxy, renders to PDF via CDP. | Renders through a filtering proxy for SSRF protection. 60s render timeout, 10s per CDP operation. Discovers browsers by platform-specific paths. |
| `resource_policy.rs` | `ResourcePolicy` — blocks non-public IPs (loopback, private, link-local, multicast). Resolves DNS before connecting. | Prevents SSRF to internal services. Blocks `file:`, `data:`, `localhost`, `127.0.0.1`, `10.x`, `169.254.x`, etc. |
| `proxy.rs` | `FilteringProxy` — local HTTP proxy that intercepts all browser resource requests and applies `ResourcePolicy`. | All Chrome traffic is forced through the proxy via `--proxy-server`. Rejected resources abort the render with `BlockedResource`. |

### Remote Polling

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `remote_client.rs` | HTTP client: `fetch_tasks`, `report_status`, `test_connection`. | `is_configuration_status()` treats 401/403/404 as config errors → backoff. |
| `remote_store.rs` | SQLite: `remote_jobs` (dedup) + `remote_status_events` (outbox). | `INSERT OR IGNORE` for idempotency. |
| `remote_worker.rs` | Infinite loop: poll → enqueue → report. Config error pauses. | Report backoff via `retry_at_string`. |

## CLI Crate (`crates/cli`)

Framework-free command types and CLI parser shared by desktop and headless products.

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `command.rs` | `Command` enum (GetConfig, SaveConfig, ListPrinters, Doctor, Status, ExportConfig, ImportConfig, etc.). Each command has a `CommandPolicy`. | Adding a command requires updating `policy()` and the executor implementations. |
| `service.rs` | `CommandService` — dispatches `Command` to online (Agent via IPC) or offline executor based on policy. | Only `NotRunning` errors trigger offline fallback for `OnlinePreferred` commands. |
| `parser.rs` | Clap command structure (status, config, printer, paper, origin, remote, task, logs, test-remote, test-print, autostart, app, service, ip, doctor). | CLI subcommands must match `Command` variants. |
| `client.rs` | `LocalClientExecutor` — sends commands via IPC to the running Agent. | |
| `output.rs` | `CommandResult`, `CommandError`, `AgentStatus`, `DoctorReport`/`DoctorCheck`/`DoctorSummary`, `ProductKind`. | Error kinds map to stable exit codes. |
| `policy.rs` | `CommandPolicy` enum (`OnlineOnly`, `OnlinePreferred`, `OfflineAllowed`). | |
| `product.rs` | `ProductCommandAdapter` trait (autostart, language). Desktop adapter enabled; headless uses `UnsupportedProductCommandAdapter`. | |
| `config_transfer.rs` | Encrypted export/import: Argon2id + AES-256-GCM, field selection, merge, preview diff. | Bearer token has special protection (empty/null → preserve). |

### History & Diagnostics

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `task_history.rs` | SQLite: `task_history_jobs` (aggregate) + `task_history_events` (append-only). | `finished_at` set only for terminal statuses. |
| `logs.rs` | `LogStore` — in-memory ring buffer (500 entries). Not persisted. | Drops oldest when full. |
| `test_print.rs` | Calibration test page generation. | Uses default printer + paper. |
| `agent_guard.rs` | TCP port probe to detect running instance. | GUI aborts startup; headless returns `RuntimeError::AlreadyRunning`. |

## App Crates

### Desktop (`apps/desktop`)

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `src-tauri/src/main.rs` | Binary entry. | |
| `src-tauri/src/lib.rs` | Tauri app setup: runtime builder, tray, command service wiring (online + offline executor), window close intercept. | Re-exports `print_bridge_runtime` and `print_bridge_core` modules. Window close hides to tray. |
| `src-tauri/src/cli.rs` | Desktop CLI dispatch. | |
| `src-tauri/src/product_cli.rs` | `DesktopProductCommandAdapter` — autostart via `auto_launch`, language setting. | macOS uses `LaunchAgent=false` (uses `.plist` instead). Linux resolves `APPIMAGE` path. |
| `src-tauri/src/commands.rs` | Tauri commands: config CRUD, export/import, test connection, logs, task history, print test. | `print_test` is settings UI origin only. |
| `src-tauri/src/tray.rs` | System tray: Open Settings, Test Print, View Logs, Restart, Launch at Startup, Quit. Localized. | `toggle_autostart` persists to config. |

### Server (`apps/server`)

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `src/main.rs` | Headless binary entry. No args → help. `serve` → `RuntimeBuilder` + `AgentRuntime::start`. Other args → shared CLI via `run_cli_from`. | Uses `UnsupportedProductCommandAdapter` (autostart managed by systemd, language fixed). |
| `src/dependencies.rs` | `preflight()` — verifies `lp`/`lpstat`/`lpoptions` (CUPS), `soffice`/`libreoffice`, and Chrome/Chromium on PATH before serve starts. | |
| `src/paths.rs` | `system_paths()` — resolves `/etc/print-bridge`, `/var/lib/print-bridge`, `/run/print-bridge`. | |
| `src/readiness.rs` | Sends `READY=1` / `STOPPING=1` to systemd via `sd_notify`. | |
| `src/signals.rs` | Handles `SIGTERM` / `SIGINT` for graceful shutdown. | |

## Frontend (`apps/desktop/src`)

| File | Responsibility | Watch Out For |
|------|---------------|---------------|
| `App.vue` (~71 KB) | Monolithic SFC with all UI: Settings, Remote, Website whitelist, IP whitelist, Tasks, About tabs. No router. | Single file contains all UI logic. Config changes that change port trigger app relaunch in production. |
| `api.ts` | All API calls via `invoke()` (Tauri commands). No direct HTTP fetch. | Must match Rust types — changes require updating both sides. |
| `types.ts` | Shared TypeScript types: `AgentConfig`, `PrinterInfo`, `PaperInfo`, `TaskHistoryJob`, `TaskHistoryEvent`. | Must match Rust structs. |
| `i18n.ts` | vue-i18n v11 (composition mode). zh-CN (default) + en. ~140 keys. | Task status/source labels are inline lookup tables in App.vue. |
| `updater.ts` | Tauri updater wrapper: check, download+install, relaunch. | Progress via `DownloadEvent` callbacks. |
| `main.ts` | App entry: creates Vue app, installs vue-i18n, mounts. | |

## Supporting Files

| Path | Purpose |
|------|---------|
| `apps/desktop/src-tauri/tauri.conf.json` | App config: identifier `com.vergil.printbridge`, window 960×680, updater endpoint |
| `apps/desktop/src-tauri/tauri.windows.conf.json` | Windows override: bundles `SumatraPDF.exe` as resource |
| `apps/desktop/src-tauri/capabilities/default.json` | Main window permissions: core, dialog, opener defaults |
| `apps/desktop/src-tauri/capabilities/desktop.json` | Desktop permissions: autostart, log, updater, process, dialog, opener |
| `apps/server/packaging/deb/control` | deb package metadata (`print-bridge-server`, depends systemd + cups-client + libreoffice) |
| `apps/server/packaging/rpm/print-bridge.spec` | RPM spec file |
| `apps/server/packaging/systemd/print-bridge.service` | systemd unit: `Type=notify`, runs as `printbridge` user, `ProtectSystem=strict` |
| `scripts/release.mjs` | Interactive release: version sync, tag check, push to `release` branch |
| `scripts/build-server-packages.sh` | Builds deb/rpm packages for the headless server |
| `scripts/patch-updater-json.mjs` | Post-release: rewrites `latest.json` asset URLs |
| `scripts/release-version.mjs` | Version validation for coordinated releases |
| `scripts/verify-config-transfer-examples.mjs` | Runs all config-transfer example self-tests |
| `.github/workflows/release.yml` | CI: builds desktop (macOS ARM+Intel, Linux, Windows) and server (deb/rpm) packages |
| `.github/workflows/sync-release-notes.yml` | Syncs release notes |
| `.github/workflows/openwiki-update.yml` | Scheduled OpenWiki doc refresh |
| `AGENTS.md` | AI agent development instructions (Chinese) |
| `docs/printbridge-technical.md` | Detailed technical documentation (Chinese) |
| `docs/printbridge-technical_en.md` | English technical documentation |
