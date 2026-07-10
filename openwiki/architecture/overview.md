# Architecture Overview

PrintBridge is a Tauri 2 desktop application with a Rust backend. The same binary serves as a GUI app (system tray resident) and a headless CLI agent (`print-bridge serve`). Both modes share identical core services: an Axum HTTP/WebSocket server, a serial print queue worker, and a remote task polling worker.

## Process Architecture

```
┌─────────────────────────────────────────────────────┐
│                   PrintBridge Process                │
│                                                      │
│  ┌──────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │  Tauri   │  │  Axum HTTP/  │  │   Print Queue │  │
│  │  GUI /   │  │  WebSocket   │  │    Worker     │  │
│  │  Tray    │  │   Server     │  │  (serial FIFO)│  │
│  └──────────┘  └──────┬───────┘  └───────┬───────┘  │
│                       │                  │           │
│  ┌──────────┐         │          ┌───────┴───────┐   │
│  │ Commands │◄────────┼──────────┤   AppState    │   │
│  │ (Tauri)  │         │          │  (shared)     │   │
│  └──────────┘         │          └───────┬───────┘   │
│                       │                  │           │
│  ┌────────────────────┴──────────┐       │           │
│  │    Remote Polling Worker      │◄──────┘           │
│  └───────────────────────────────┘                   │
│                                                      │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────┐  │
│  │ config.json │  │task_history  │  │ remote     │  │
│  │             │  │  .sqlite3    │  │  .sqlite3  │  │
│  └─────────────┘  └──────────────┘  └────────────┘  │
└─────────────────────────────────────────────────────┘
```

## Entry Points

### GUI Mode (`src-tauri/src/lib.rs`, `main.rs`)

`main.rs` checks `argv[1]` — if it matches a known CLI command (`serve`, `printer`, `paper`, `origin`, `remote`, `task`, `--help`), it dispatches to the CLI and exits. Otherwise, it calls `print_bridge_lib::run()`.

`run()` in `lib.rs` performs the Tauri setup:
1. Resolves config directory, creates it, loads `AgentConfig`
2. **Agent guard check** — probes TCP port; aborts startup if another PrintBridge instance is already running
3. Sets up system tray (`tray.rs`)
4. Resolves the platform print backend (Windows: bundled SumatraPDF; macOS/Linux: CUPS)
5. Opens `remote.sqlite3` and `task_history.sqlite3`
6. Builds `AppState` and registers it with Tauri via `app.manage(state)`
7. Spawns three async Tokio tasks: server, queue worker, remote worker
8. Intercepts main window close → hides to tray instead of quitting

Registered Tauri commands: `get_config`, `save_config`, `export_config_file`, `preview_config_import`, `import_config_file`, `test_remote_connection`, `get_logs`, `get_task_history`, `get_task_history_events`, `clear_task_history`, `is_debug_build`, `print_test`.

### Headless Mode (`src-tauri/src/runtime.rs`)

`run_headless()` performs the same core setup without Tauri:
1. Resolves config path + data dir from environment
2. Loads config
3. **Agent guard check** — returns `AlreadyRunning` error if port is occupied (exits cleanly with exit code 0)
4. Binds TCP listener via `server::bind_listener`
5. Builds `AppState`
6. Spawns the same three async tasks
7. Blocks on `tokio::select!` — server task join or shutdown signal (SIGTERM/Ctrl+C)

## Shared State (`app_state.rs`)

`AppState` is the central runtime container shared across all async tasks and Tauri commands:

| Field | Type | Purpose |
|-------|------|---------|
| `config` | `Arc<RwLock<AgentConfig>>` | Thread-safe config; read by all workers, written by CLI/commands |
| `config_path` | `PathBuf` | Path to `config.json` |
| `logs` | `Arc<Mutex<LogStore>>` | In-memory ring buffer (500 entries, not persisted) |
| `status_events` | `broadcast::Sender<TaskLogEntry>` | Fan-out to WebSocket connections (capacity 128) |
| `queue` | `Arc<Mutex<QueueState>>` | FIFO print queue with dedup |
| `queue_notify` | `Arc<Notify>` | Wakes the queue worker when jobs are enqueued |
| `remote_notify` | `Arc<Notify>` | Wakes the remote worker when config changes |
| `print_lock` | `Arc<Mutex<()>>` | Serializes actual print execution |
| `printing` | `Arc<dyn PrintBackend>` | Platform print backend (SumatraPDF or CUPS) |
| `remote_store` | `RemoteStore` | SQLite-backed remote dedup + status outbox |
| `task_history` | `TaskHistoryStore` | SQLite-backed task history |

## Agent Guard (`agent_guard.rs`)

GUI and `serve` are **mutually exclusive**. Only one PrintBridge instance can run per machine.

`check_agent_port(config)`:
1. TCP connects to `127.0.0.1:<port>` with 300ms timeout
2. If connection fails → `Available` (safe to start)
3. If connected, sends `GET /health HTTP/1.1`
4. If response is HTTP 200 with `{"service":"print-bridge",...}` → `PrintBridge` (another instance is running)
5. Any other response → `OccupiedByOther` (something else is on the port)

Consumers:
- **GUI** (`lib.rs`): Returns `AlreadyExists` error → aborts app startup
- **Headless** (`runtime.rs`): Returns `AlreadyRunning` → prints friendly message, exits 0

## Three Async Tasks

Both GUI and headless modes spawn identical Tokio tasks:

### 1. Server Task
`server::serve_listener()` — runs the Axum HTTP/WebSocket server on the bound TCP listener. Handles REST endpoints and WebSocket connections. See [Protocol & API](protocol.md).

### 2. Queue Worker
`queue::run_worker()` — infinite loop that pops jobs from the FIFO queue one at a time, processes each fully before the next, and sleeps on `queue_notify` when idle. See [Printing Pipeline](../workflows/printing-pipeline.md).

### 3. Remote Worker
`remote_worker::run_worker()` — if `remote.enabled`, periodically polls the endpoint URL for tasks and delivers pending status reports. Sleeps on `remote_notify` when disabled or on config error. See [Remote Task Polling](../workflows/remote-polling.md).

## Platform Abstraction

Print execution is abstracted behind the `PrintBackend` trait (`printing/mod.rs`). The concrete implementation is selected at compile time:

| Platform | Backend | PDF Printing | Raw Printing | Job Tracking |
|----------|---------|-------------|-------------|--------------|
| Windows | `WindowsPrintBackend` | SumatraPDF CLI (`-silent -print-to`) | Win32 Spooler API (`OpenPrinterW` → `WritePrinter`) | Not supported |
| macOS | `CupsPrintBackend` (macos) | `lp -d "{printer}" -n {copies} -o media={media}` | `lp -d "{printer}" -o raw` | Yes (`lpstat -W completed`) |
| Linux | `CupsPrintBackend` (linux) | Same as macOS | Same as macOS | Yes |

## Data Storage

| File | Format | Purpose |
|------|--------|---------|
| `config.json` | JSON | Agent configuration (shared by GUI + CLI) |
| `task_history.sqlite3` | SQLite | Job history + event log |
| `remote.sqlite3` | SQLite | Remote task dedup + status report outbox |

Default data directories:

| Platform | Path |
|----------|------|
| Windows | `%APPDATA%\com.vergil.printbridge` |
| macOS | `~/Library/Application Support/com.vergil.printbridge` |
| Linux | `${XDG_CONFIG_HOME:-~/.config}/com.vergil.printbridge` |

Overridable via `PRINT_BRIDGE_DATA_DIR` and `PRINT_BRIDGE_CONFIG_PATH` environment variables. See [Configuration](../domain/configuration.md).

## Source References

| Area | File |
|------|------|
| Binary entry / CLI dispatch | `src-tauri/src/main.rs` |
| Tauri app setup | `src-tauri/src/lib.rs` |
| Headless runtime | `src-tauri/src/runtime.rs` |
| Shared state container | `src-tauri/src/app_state.rs` |
| Agent guard | `src-tauri/src/agent_guard.rs` |
| System tray | `src-tauri/src/tray.rs` |
| Platform print dispatch | `src-tauri/src/printing/mod.rs` |
