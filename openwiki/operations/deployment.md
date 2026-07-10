# Operations & Deployment

PrintBridge supports three deployment modes: GUI desktop app (default), headless `serve` mode for unattended operation, and CLI for configuration and diagnostics. This page covers CLI commands, headless service management, troubleshooting, and platform-specific guidance.

## CLI Reference

The `print-bridge` CLI operates without the GUI, without Tauri CLI plugins, and without requiring the agent service to be running. It reads and writes the same `config.json` and reads `task_history.sqlite3`.

```bash
# Printer management
print-bridge printer                          # List printers (* = default)
print-bridge printer "Printer Name"           # Show details for a printer
print-bridge printer set-default "Printer Name"

# Paper management
print-bridge paper                            # Show default paper
print-bridge paper set 60 40                  # Set default paper (width_mm height_mm)

# Origin allowlist
print-bridge origin                           # List allowed origins
print-bridge origin add "https://example.com"
print-bridge origin delete "https://example.com"

# Remote task polling
print-bridge remote                           # Show remote config
print-bridge remote enable
print-bridge remote disable
print-bridge remote set-url "https://example.com/print-task"
print-bridge remote set-token "token"
print-bridge remote set-device-id "factory-pi-01"
print-bridge remote set-device-name "packing-station-01"
print-bridge remote set-interval 10

# Task history
print-bridge task                             # Recent tasks
print-bridge task "JOB-001"                   # Task detail with events
print-bridge task clear                       # Clear all history

# Headless serve
print-bridge serve                            # Start agent in foreground
print-bridge serve install                    # Install as managed service (Linux/macOS)
print-bridge serve uninstall                  # Remove managed service (Linux/macOS)
```

All config-modifying commands load → modify → save atomically. Empty strings for token/device fields are converted to `None`.

## Headless Serve Mode

`print-bridge serve` starts the full agent (HTTP/WS server + queue worker + remote worker) as a foreground process without the GUI. It prints config path, data directory, and listen address on startup:

```
PrintBridge serve started
config: /path/to/config.json
data: /path/to/data
listen: 0.0.0.0:17890
```

### GUI / Serve Mutual Exclusivity

Only one PrintBridge instance can run per machine. If another instance is already running on the same port, the second entry exits cleanly. See [Architecture → Agent Guard](../architecture/overview.md#agent-guard).

### When to Use Serve vs GUI

| Scenario | Recommended Mode |
|----------|-----------------|
| Windows/macOS desktop | GUI (tray, window, auto-update) |
| Manual debugging / temporary | `print-bridge serve` (foreground, logs to terminal) |
| Linux headless (Raspberry Pi, IPC) | systemd user service |
| macOS background (user session) | launchd LaunchAgent |
| Windows unattended | GUI resident, or service wrapper (WinSW/NSSM) with careful validation |

## Managed Service Installation

### Linux — systemd user service

```bash
print-bridge serve install
```

Creates `~/.config/systemd/user/print-bridge.service` with:
- `ExecStart=<exe> serve`
- `Restart=on-failure`, `RestartSec=3`
- `PRINT_BRIDGE_DATA_DIR` and `PRINT_BRIDGE_CONFIG_PATH` injected

Runs `systemctl --user daemon-reload` and `systemctl --user enable --now`.

```bash
# Check status
systemctl --user status print-bridge.service
journalctl --user -u print-bridge.service -f

# Remove
print-bridge serve uninstall
```

To run without a login session, an admin must enable linger:

```bash
sudo loginctl enable-linger "$USER"
```

**Linux requires CUPS.** Verify the printing user can see and use the target printer:

```bash
lpstat -e
lpoptions -d
echo "PrintBridge test" | lp
```

### macOS — launchd LaunchAgent

```bash
print-bridge serve install
```

Creates `~/Library/LaunchAgents/com.printbridge.agent.plist` with `RunAtLoad=true`, `KeepAlive=true`. Logs go to `~/Library/Logs/printbridge.log` and `printbridge.err.log`.

```bash
# Check
launchctl print "gui/$(id -u)/com.printbridge.agent"
tail -f ~/Library/Logs/printbridge.log ~/Library/Logs/printbridge.err.log

# Remove
print-bridge serve uninstall
```

LaunchAgent (not LaunchDaemon) is recommended — it runs in the user session, with better access to the user's printers and keychain.

### Windows — Not Supported via CLI

`serve install` and `serve uninstall` are not available on Windows. Regular desktop deployments should use the GUI. For unattended operation, use WinSW, NSSM, or a similar service wrapper — but validate that the service account can see the target printers first.

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `PRINT_BRIDGE_DATA_DIR` | Overrides the data directory (config + SQLite databases) |
| `PRINT_BRIDGE_CONFIG_PATH` | Overrides only the config file path |

See [Configuration](../domain/configuration.md) for default data directory paths per platform.

## Troubleshooting Checklist

### Service won't start

1. **Port in use?** — Check if another process occupies port 17890
2. **Config valid?** — Validate `config.json` is well-formed JSON
3. **Paths writable?** — Verify `PRINT_BRIDGE_CONFIG_PATH` and `PRINT_BRIDGE_DATA_DIR` are read/write accessible
4. **CUPS installed?** (Linux/macOS) — Ensure `lpstat`, `lpoptions`, `lp` are available
5. **Printer visible?** — Run `print-bridge printer` as the service user
6. **Allowlists correct?** — Verify Origin and IP allowlists permit the caller

### Quick diagnostics

```bash
curl http://127.0.0.1:17890/health
print-bridge printer
print-bridge task
```

### Rust test failures

Some Rust tests bind local TCP ports. If tests fail due to sandbox, security software, or missing printers, verify in a normal terminal first — the failure may be environmental, not a code issue.

## Development Commands

```bash
# Frontend
pnpm install
pnpm typecheck        # vue-tsc --noEmit
pnpm lint             # oxlint
pnpm format:check     # oxfmt
pnpm build            # typecheck + vite build

# Desktop dev
pnpm tauri dev        # Starts Vite dev server (port 1420) + Tauri

# Rust checks
cd src-tauri
cargo fmt --check
cargo check
cargo clippy --tests -- -D warnings
cargo test

# Config transfer examples
pnpm verify:config-transfer-examples

# Release
pnpm release          # Interactive release script
pnpm release:app      # App-only release
```

## Source References

| Area | File |
|------|------|
| CLI command structure + handlers | `src-tauri/src/cli.rs` |
| Headless runtime | `src-tauri/src/runtime.rs` |
| Service manager (systemd/launchd) | `src-tauri/src/service_manager.rs` |
| Agent guard (exclusivity) | `src-tauri/src/agent_guard.rs` |
| Release scripts | `scripts/release.mjs` |
| CI workflows | `.github/workflows/release.yml` |
