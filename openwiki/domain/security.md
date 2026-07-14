# 安全模型

PrintBridge 运行在用户的本地电脑上，可以访问本地打印机。安全通过双层白名单架构（IP + Origin）、用于部署的加密配置传输和清晰的信任边界来保障。

## 双层白名单架构

所有请求必须通过**两层**访问控制：

```
传入请求（HTTP 或 WebSocket）
    │
    ├── 第 1 层：IP 白名单中间件（所有路由）
    │   从 Axum ConnectInfo<SocketAddr> 提取客户端 IP
    │   → 不在 allowed_ips 中则拒绝
    │
    └── 第 2 层：WebSocket Origin 检查（仅 /ws）
        根据 allowed_origins 校验 Origin 头
        → Origin 不在列表中则拒绝 WebSocket 升级
```

### IP 白名单（`ip_whitelist.rs`）

- **`127.0.0.1` 始终存在且不可移除**（`REQUIRED_LOOPBACK_IP`）
- 所有回环 IP（`is_loopback()`）无论列表如何始终允许
- 支持单个 IPv4/IPv6 地址和 CIDR 范围（通过 `ipnet` crate）
- **拒绝：** `0.0.0.0`、`::`、`0.0.0.0/0`、`::/0` 以及任何 `/0` 前缀——不允许允许所有条目
- 客户端 IP 仅从 Axum `ConnectInfo` 提取——**不信任 `X-Forwarded-For` 和其他代理头**
- 不支持通配符或范围语法——范围请使用 CIDR 表示法

### Origin 白名单（`config.security.allowed_origins`）

- 校验浏览器页面的 `Origin` 头（协议 + 主机 + 端口）
- **必须精确匹配**——`https://example.com` ≠ `https://example.com:443` ≠ `http://example.com`
- 仅校验**连接页面的 origin**，而非文件 URL 的域名
- 在 WebSocket 握手时应用（`server.rs`、`ws_handler`）

## 网络绑定

服务器绑定到 `0.0.0.0:{port}`——所有网络接口。这允许局域网设备连接到代理。配置中的 `service.host` 字段是兼容占位符（`127.0.0.1`），不控制绑定。

**安全影响：** 如果机器位于不受信任的网络上，服务端口是可达的。IP 白名单是主要防线——始终保持其限制性。

## 配置传输加密（`crates/cli/src/config_transfer.rs`）

配置导出/导入使用认证加密，用于工作站配置的安全批量部署。

### 加密参数

| 参数 | 值 |
|------|-----|
| KDF | Argon2id v19 |
| 内存 | 19,456 KiB |
| 迭代次数 | 2 |
| 并行度 | 1 |
| 加密算法 | AES-256-GCM |
| 密钥 | 32 字节 |
| 盐值 | 16 字节（随机，`OsRng`） |
| Nonce | 12 字节（随机，`OsRng`） |
| 认证标签 | 16 字节 |
| 密钥材料 | 使用后清零 |

### 加密信封格式

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

### 导出流程

1. 选择要包含的配置段（端口、Origin、远程设置等）——每个都是布尔开关（`ExportConfigOptions`）
2. 构建仅包含所选字段的 `ConfigTransferPayload`
3. 生成随机盐值 + nonce，通过 Argon2id 派生密钥
4. 使用 AES-256-GCM 加密 JSON 负载
5. 将信封写入文件（默认文件名：`printbridge-config.json`）
6. 计算文件内容的 SHA-256 哈希，用于导入确认

密码可以为空——空密码仍然经过完整的加密流程。如果 Authorization Token 被包含在内且密码为空，UI 会要求确认。

### 导入流程

1. 读取加密文件 + 计算 SHA-256 哈希（用于用户确认）
2. 用密码解密
3. **预览**——显示每个字段的当前值 → 新值 diff（`ImportPreview`）
4. 用户确认后 → **逐字段合并**到当前配置
5. 仅文件中存在的字段被覆盖；不存在的字段保留当前值
6. 保存 `normalized()` 配置

**Bearer token 保护：** 如果导入文件的 token 字段缺失、为 `null` 或空字符串 → 保留当前 token。仅非空字符串才会覆盖。

### 跨语言生成

ERP 或部署系统可以在不使用 PrintBridge 的情况下生成可导入的配置文件。`examples/config-transfer/` 中的参考实现了相同的加密格式：

| 语言 | 库 |
|------|-----|
| Node.js | `libsodium-wrappers-sumo` + Node `crypto` |
| PHP | `sodium_crypto_pwhash` + `openssl_encrypt` |
| Go | `golang.org/x/crypto/argon2` + `crypto/aes` |

所有实现共享相同的常量和交叉验证的测试向量。使用以下命令验证：

```bash
pnpm verify:config-transfer-examples
```

## 信任边界与最佳实践

- 仅向 Origin 白名单添加**受信任的业务系统**
- 仅向 IP 白名单添加**受信任的客户端 IP/CIDR 范围**
- 即使服务监听在局域网上，也**不要**将端口暴露给不受信任的网络
- 在**业务系统端**控制谁可以创建打印任务以及哪些文件可以被打印
- **不要**将敏感文件 URL 暴露给不受信任的页面——Origin 检查不校验文件 URL 域名
- `service.host` 字段是装饰性的——始终将服务视为监听所有接口

## 源码参考

| 领域 | 文件 |
|------|------|
| IP 白名单逻辑 | `crates/core/src/ip_whitelist.rs` |
| 配置加密/解密 | `crates/cli/src/config_transfer.rs` |
| WebSocket 中间件（IP 检查） | `crates/runtime/src/server.rs`（`ip_whitelist_middleware`） |
| WebSocket Origin 校验 | `crates/runtime/src/server.rs`（`ws_handler`） |
| 配置归一化（强制 127.0.0.1、IP 归一化） | `crates/core/src/config.rs` |
