# PrintBridge — Quick Start

PrintBridge is a local print agent that runs on the user's computer. It lets trusted web pages or remote business servers send PDF files, images, Office documents, and raw printer commands to the local system print queue — for labels, shipping documents, receipts, reports, and other business scenarios that need reliable silent printing.

It does **not** replace printer drivers and does **not** bypass the OS print queue. It receives tasks, validates the source, downloads or converts files, and submits jobs to the local operating system. Actual paper output is still handled by the system print queue, printer driver, and hardware.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Framework | Tauri 2 |
| Frontend | Vue 3 + TypeScript |
| UI | shadcn-vue + Tailwind CSS |
| Build | Vite |
| Backend | Rust + Axum + Tokio |
| Storage | JSON config + SQLite |
| Office conversion | office2pdf crate |
| Platform printing | SumatraPDF (Windows) / CUPS `lp` (macOS/Linux) |

## How It Works

```
Web page / remote business server
  │
  ├── WebSocket /ws (browser → agent, real-time)
  │   or
  ├── HTTP polling (remote server → agent, periodic)
  │
  ▼
PrintBridge (validate source → download → convert → serial queue)
  │
  ▼
System print queue → printer driver → printer
```

`submitted` / `success` means the job was accepted by the OS print queue. It does **not** mean the printer has physically finished output.

## Getting Started (Development)

```bash
pnpm install
pnpm tauri dev
```

Tauri starts the Vite dev server on `http://localhost:1420/`. The local print service listens on `0.0.0.0:17890`.

## Key Capabilities

- System tray resident, main window hidden by default
- Cross-platform: Windows, macOS, Linux
- Local HTTP/WebSocket service (default port `17890`)
- Website Origin allowlist + IP/CIDR allowlist (dual-layer security)
- Supports PDF, PNG/JPEG images, Office (docx/xlsx/pptx), and raw printer commands (ESC/POS, TSPL, ZPL, EPL, PCL, PostScript)
- Serial print queue — no concurrent printer contention
- Remote task polling for unattended workstations
- CLI operations mode (`print-bridge serve`, `print-bridge printer`, etc.)
- Encrypted config export/import for batch deployment
- In-app online updates

## Documentation Map

### [Architecture](architecture/overview.md)
How the Tauri app, Axum server, print queue worker, and remote polling worker fit together. Covers entry points, shared `AppState`, and agent guard exclusivity.

### [Protocol & API](architecture/protocol.md)
WebSocket print protocol (message types, job lifecycle, status events, error codes) and HTTP REST endpoints. Essential for integration.

### [Printing Pipeline](workflows/printing-pipeline.md)
How jobs flow through the serial queue: download → format detection → conversion (Office/Image → PDF) → platform-specific print execution (SumatraPDF on Windows, CUPS `lp` on macOS/Linux).

### [Remote Task Polling](workflows/remote-polling.md)
Server-initiated task delivery: the poll/report HTTP protocol, dedup via SQLite, outbox-based status reporting with exponential backoff, and configuration error handling.

### [Security Model](domain/security.md)
Dual allowlist architecture (IP + Origin), config transfer encryption (Argon2id + AES-256-GCM), and security best practices.

### [Configuration](domain/configuration.md)
Complete config structure (`service`, `security`, `printing`, `limits`, `app`, `remote`), data directory paths, environment variables, and field reference.

### [Operations & Deployment](operations/deployment.md)
CLI command reference, headless `serve` mode, systemd/launchd service management, troubleshooting, and platform-specific deployment guidance.

### [Source Map](source-map.md)
Developer's guide to the codebase: file-by-file responsibilities, what to watch out for, and key design decisions.

## Integration Points

| Consumer | Protocol | Reference |
|----------|----------|-----------|
| Browser web pages | WebSocket `/ws` | [Protocol & API](architecture/protocol.md) — or use [print-bridge-sdk](https://github.com/vergil-lai/print-bridge-jssdk) |
| Business servers | HTTP polling | [Remote Task Polling](workflows/remote-polling.md) — see `examples/remote-task/` for server examples |
| ERP/batch deployment | Encrypted config | [Security Model](domain/security.md) — see `examples/config-transfer/` for cross-language generators |
| Settings UI / diagnostics | HTTP REST | [Protocol & API](architecture/protocol.md#http-api) |

## Primary Source Files

Quick reference for the most important entry points:

| Area | Path |
|------|------|
| App entry (GUI) | `src-tauri/src/lib.rs` |
| Headless entry | `src-tauri/src/runtime.rs` |
| HTTP/WS server | `src-tauri/src/server.rs` |
| WebSocket protocol | `src-tauri/src/protocol.rs` |
| Print queue + pipeline | `src-tauri/src/queue.rs` |
| Config | `src-tauri/src/config.rs` |
| CLI | `src-tauri/src/cli.rs` |
| Frontend | `src/App.vue` |

## Existing Documentation

| Document | Language |
|----------|----------|
| `README.md` | Chinese |
| `README_en.md` | English |
| `docs/printbridge-technical.md` | Chinese (detailed protocol, API, config, deployment) |
| `docs/printbridge-technical_en.md` | English |
| `AGENTS.md` | AI agent instructions (Chinese) |
