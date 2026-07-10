# Configuration

PrintBridge stores all settings in a single JSON config file shared by the GUI, CLI, headless `serve`, remote worker, and config import/export. The same file is the single source of truth — CLI commands modify it directly, the GUI reads/writes it via Tauri commands.

## Config Structure

The root `AgentConfig` (`config.rs`) has six sections:

### `service`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `host` | String | `"127.0.0.1"` | Compatibility field only — does not control binding. Always normalized to `127.0.0.1`. |
| `port` | u16 | `17890` | Actual listen port. Range: 10000–65535. |

### `security`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `allowed_origins` | Vec\<String\> | `[]` | Website Origin allowlist for WebSocket connections |
| `allowed_ips` | Vec\<String\> | `["127.0.0.1"]` | IP/CIDR allowlist. `127.0.0.1` is always present and cannot be removed. |

### `printing`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `default_printer` | Option\<String\> | `null` | Default printer name |
| `default_paper` | Option\<EffectivePaper\> | `null` | Default paper (width_mm + height_mm) |
| `default_copies` | u16 | `1` | Default copy count |

### `limits`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `max_file_size_mb` | u32 | `20` | Max file size per download |
| `max_batch_jobs` | u32 | `20` | Max jobs per batch |
| `max_copies` | u16 | `100` | Max copies per job |
| `download_timeout_seconds` | u64 | `30` | Download timeout |

### `app`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `autostart` | bool | `false` | Launch at system startup |
| `language` | UiLanguage | `zh-CN` | UI language (`zh-CN` or `en`) |

### `remote`

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `enabled` | bool | `false` | Enable remote task polling |
| `endpoint_url` | Option\<String\> | `null` | Poll/report endpoint URL (http/https only) |
| `bearer_token` | Option\<String\> | `null` | Authorization Bearer token |
| `device_id` | Option\<String\> | `null` | Sent as `X-PrintBridge-Device-Id` header |
| `device_name` | Option\<String\> | `null` | Sent as `X-PrintBridge-Device-Name` header |
| `poll_interval_seconds` | u64 | `10` | Poll interval (minimum 3) |
| `max_report_retries` | u32 | `10` | Status report retry limit (minimum 1) |
| `history_retention_days` | u32 | `3` | Remote state retention period |

## Example Config

```json
{
  "service": { "host": "127.0.0.1", "port": 17890 },
  "security": {
    "allowed_origins": ["https://example.com"],
    "allowed_ips": ["127.0.0.1", "192.168.1.0/24"]
  },
  "printing": {
    "default_printer": "TSC TE244",
    "default_paper": { "width_mm": 60, "height_mm": 40 },
    "default_copies": 1
  },
  "limits": {
    "max_file_size_mb": 20,
    "max_batch_jobs": 20,
    "max_copies": 100,
    "download_timeout_seconds": 30
  },
  "app": { "autostart": false, "language": "zh-CN" },
  "remote": {
    "enabled": false,
    "endpoint_url": null,
    "bearer_token": null,
    "device_id": null,
    "device_name": null,
    "poll_interval_seconds": 10,
    "max_report_retries": 10,
    "history_retention_days": 3
  }
}
```

## Persistence and Loading

- `AgentConfig::load(path)` — reads JSON; returns `Default` if file doesn't exist; always calls `normalized()` (forces `host=127.0.0.1`, normalizes IP whitelist)
- `AgentConfig::save(path)` — pretty-prints JSON, creates parent directories
- Both GUI and CLI use the same `config.json` file

## Data Directories

| Platform | Default Path |
|----------|-------------|
| Windows | `%APPDATA%\com.vergil.printbridge` |
| macOS | `~/Library/Application Support/com.vergil.printbridge` |
| Linux | `${XDG_CONFIG_HOME:-~/.config}/com.vergil.printbridge` |

Files within the data directory:

| File | Purpose |
|------|---------|
| `config.json` | Agent configuration |
| `task_history.sqlite3` | Task history + event log |
| `remote.sqlite3` | Remote task dedup + status outbox |

## Environment Variables

| Variable | Effect |
|----------|--------|
| `PRINT_BRIDGE_DATA_DIR` | Overrides the entire data directory (config + SQLite databases) |
| `PRINT_BRIDGE_CONFIG_PATH` | Overrides only the config file path; SQLite databases still go to the data directory |

If `PRINT_BRIDGE_CONFIG_PATH` is not set, config defaults to `{data_dir}/config.json`.

## Validation Rules

The config system enforces these constraints:

- Port range: 10,000–65,535
- Remote poll interval: ≥ 3 seconds
- Remote report retries: ≥ 1
- Remote endpoint URL: http or https only
- IP entries: valid IPv4/IPv6 or CIDR; rejects `0.0.0.0`, `::`, `/0` prefixes
- Origins: must be valid URLs with http/https scheme
- `127.0.0.1` is always injected into `allowed_ips`

## Config Import/Export

See [Security Model](security.md) for the encrypted config transfer format and cross-language generation examples. The settings UI supports:
- **Export:** select which sections to include, set a password (optional), generate encrypted `printbridge-config.json`
- **Import:** select file + password, preview the diff, confirm merge
- Bearer token is only overwritten if the import file contains a non-empty string

## Source References

| Area | File |
|------|------|
| Config structs + defaults + load/save | `src-tauri/src/config.rs` |
| Config encryption/decryption | `src-tauri/src/config_transfer.rs` |
| Data dir resolution + env vars | `src-tauri/src/config.rs` (`cli_data_dir`, `cli_config_path`) |
| Frontend config type | `src/types.ts` |
| Frontend API calls | `src/api.ts` |
