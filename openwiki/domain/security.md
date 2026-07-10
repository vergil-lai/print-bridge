# Security Model

PrintBridge runs on the user's local computer and can access local printers. Security is enforced through a dual-allowlist architecture (IP + Origin), encrypted config transfer for deployment, and clear trust boundaries.

## Dual Allowlist Architecture

All requests must pass **both** layers of access control:

```
Incoming request (HTTP or WebSocket)
    │
    ├── Layer 1: IP Whitelist Middleware (all routes)
    │   Extract client IP from Axum ConnectInfo<SocketAddr>
    │   → Reject if not in allowed_ips
    │
    └── Layer 2: WebSocket Origin Check (/ws only)
        Validate Origin header against allowed_origins
        → Reject WebSocket upgrade if Origin not in list
```

### IP Whitelist (`ip_whitelist.rs`)

- **`127.0.0.1` is always present and cannot be removed** (`REQUIRED_LOOPBACK_IP`)
- All loopback IPs (`is_loopback()`) are always allowed regardless of the list
- Supports individual IPv4/IPv6 addresses and CIDR ranges (via `ipnet` crate)
- **Rejects:** `0.0.0.0`, `::`, `0.0.0.0/0`, `::/0`, and any `/0` prefix — no allow-all entries
- Client IP is extracted from Axum `ConnectInfo` only — **`X-Forwarded-For` and other proxy headers are not trusted**
- No wildcard or range syntax — use CIDR notation for ranges

### Origin Allowlist (`config.security.allowed_origins`)

- Validates the browser page's `Origin` header (protocol + host + port)
- **Must match exactly** — `https://example.com` ≠ `https://example.com:443` ≠ `http://example.com`
- Only validates the **connecting page's origin**, not the file URL's domain
- Applied at WebSocket handshake time (`server.rs`, `ws_handler`)

## Network Binding

The server binds to `0.0.0.0:{port}` — all network interfaces. This allows LAN devices to connect to the agent. The `service.host` field in config is a compatibility placeholder (`127.0.0.1`) and does not control binding.

**Security implication:** If the machine is on an untrusted network, the service port is reachable. The IP whitelist is the primary defense — always keep it restrictive.

## Config Transfer Encryption (`config_transfer.rs`)

Config export/import uses authenticated encryption for secure batch deployment of workstation configurations.

### Crypto Parameters

| Parameter | Value |
|-----------|-------|
| KDF | Argon2id v19 |
| Memory | 19,456 KiB |
| Iterations | 2 |
| Parallelism | 1 |
| Cipher | AES-256-GCM |
| Key | 32 bytes |
| Salt | 16 bytes (random, `OsRng`) |
| Nonce | 12 bytes (random, `OsRng`) |
| Auth tag | 16 bytes |
| Key material | Zeroized after use |

### Encrypted Envelope Format

```json
{
  "format": "printbridge-config-encrypted",
  "version": 1,
  "crypto": {
    "kdf": "argon2id13",
    "memory_kib": 19456,
    "iterations": 2,
    "parallelism": 1,
    "cipher": "aes-256-gcm",
    "tag_bytes": 16,
    "salt": "<base64>",
    "nonce": "<base64>"
  },
  "payload": "<base64(ciphertext || tag)>"
}
```

### Export Flow

1. Select config sections to include (port, origins, remote settings, etc.) — each is a boolean toggle (`ExportConfigOptions`)
2. Build `ConfigTransferPayload` with only selected fields
3. Generate random salt + nonce, derive key via Argon2id
4. Encrypt JSON payload with AES-256-GCM
5. Write envelope to file (default name: `printbridge-config.json`)
6. Compute SHA-256 hash of file content for import confirmation

The password can be empty — an empty password still goes through the full encryption pipeline. If the Authorization Token is included with an empty password, the UI requires confirmation.

### Import Flow

1. Read encrypted file + compute SHA-256 hash (for user confirmation)
2. Decrypt with password
3. **Preview** — show a diff of current → new values for each field (`ImportPreview`)
4. On user confirmation → **field-by-field merge** into current config
5. Only fields present in the file are overwritten; absent fields retain current values
6. Save `normalized()` config

**Bearer token protection:** If the import file's token field is missing, `null`, or empty string → the current token is preserved. Only a non-empty string overwrites.

### Cross-Language Generation

ERP or deployment systems can generate importable config files without PrintBridge. Reference implementations in `examples/config-transfer/` implement the same crypto format:

| Language | Library |
|----------|---------|
| Node.js | `libsodium-wrappers-sumo` + Node `crypto` |
| PHP | `sodium_crypto_pwhash` + `openssl_encrypt` |
| Go | `golang.org/x/crypto/argon2` + `crypto/aes` |

All share identical constants and a cross-verified test vector. Verify with:

```bash
pnpm verify:config-transfer-examples
```

## Trust Boundaries and Best Practices

- Add **only trusted business systems** to the Origin allowlist
- Add **only trusted client IPs/CIDR ranges** to the IP allowlist
- Even when the service listens on LAN, **do not expose the port to untrusted networks**
- Control who can create print tasks and which files can be printed **on the business system side**
- **Do not** expose sensitive file URLs to untrusted pages — the Origin check does not validate file URL domains
- The `service.host` field is cosmetic — always treat the service as listening on all interfaces

## Source References

| Area | File |
|------|------|
| IP whitelist logic | `src-tauri/src/ip_whitelist.rs` |
| Config encryption/decryption | `src-tauri/src/config_transfer.rs` |
| HTTP middleware (IP check) | `src-tauri/src/server.rs` (`ip_whitelist_middleware`) |
| WebSocket Origin validation | `src-tauri/src/server.rs` (`ws_handler`) |
| Config normalization (forced 127.0.0.1, IP normalization) | `src-tauri/src/config.rs` |
