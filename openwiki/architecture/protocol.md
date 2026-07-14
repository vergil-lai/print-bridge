# 打印协议与 API

PrintBridge 暴露一个 WebSocket 端点用于浏览器发起的实时打印。配置、日志、打印机发现和诊断通过 Tauri 命令（桌面）或 CLI/本地 IPC（headless）处理——没有 HTTP REST API。

WebSocket 协议由 Axum 服务器在 `0.0.0.0:{port}`（默认 `17890`）上提供。路由仅暴露 `/ws`；没有其他 HTTP 路由可用。

## WebSocket API

### 连接

```
ws://127.0.0.1:17890/ws
```

在 WebSocket 握手期间，服务器根据 `config.security.allowed_origins` 校验 `Origin` 头。如果 Origin 不在白名单中，升级请求将被拒绝。连接的客户端 IP 也必须通过 IP 白名单检查（作为中间件应用于包括 `/ws` 在内的所有路由）。

### 客户端 → 服务器消息

所有消息均为 JSON，带有 `type` 区分字段（`#[serde(tag = "type")]`）。

| 消息类型 | 字段 | 用途 |
|---------|------|------|
| `ping` | `time` | 心跳；服务器回复 `pong` |
| `get_printers_list` | — | 请求可用打印机列表 |
| `get_printer_info` | `printer_name` | 请求特定打印机的详情 |
| `get_print_queue` | — | 请求当前打印队列状态 |
| `print` | `request_id`、`job`（format、`file_url`/`data_base64`/`html`、`printer_name?`、`copies?`、`paper?`、`wait_ms?`） | 提交单个打印作业 |
| `print_batch` | `request_id`、`batch_id`、`jobs[]` | 原子化提交多个作业 |

### 服务器 → 客户端消息

| 消息类型 | 用途 |
|---------|------|
| `pong` | 心跳响应，附带 `time` |
| `printers_list` | 对 `get_printers_list` 的响应 |
| `printer_info` | 对 `get_printer_info` 的响应 |
| `print_queue` | 对 `get_print_queue` 的响应 |
| `job_status` | 作业状态更新（异步推送） |
| `error` | 错误响应，附带 `code` 和 `message` |

### 作业生命周期

每个 WebSocket 连接只接收其提交作业的 `job_status` 事件（通过每连接 `accepted_job_ids: HashSet` 跟踪）。

```
Queued → Downloading → Printing → Submitted → Completed
                                          ↘ Failed
                                          ↘ Unknown
                                          ↘ Cancelled
```

| 状态 | 含义 |
|------|------|
| `queued` | 作业已进入串行队列 |
| `downloading` | 正在从 `file_url` 下载文件 |
| `printing` | 正在转换（如需要）并提交到操作系统打印队列 |
| `submitted` | 作业已被操作系统打印队列接受（状态报告的终态） |
| `completed` | 操作系统报告作业已完成（仅 CUPS；Windows 无法跟踪） |
| `failed` | 作业在任何阶段失败 |
| `unknown` | 操作系统无法确定最终状态 |
| `cancelled` | 作业已取消 |

### 单个打印作业示例

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

- `printer_name` 可选——回退到默认打印机
- `paper` 可选——回退到默认纸张
- `copies` 可选——默认 1；必须 ≤ `limits.max_copies`

### Raw 打印作业示例

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

Raw 作业**不**支持 `file_url`、`paper` 或 `copies`。`data_base64` 字节原样提交到操作系统打印队列。PrintBridge 不解析或生成设备命令（ESC/POS、TSPL、ZPL、EPL、PCL、PostScript）。

### HTML 打印作业示例

```json
{
  "type": "print",
  "request_id": "REQ-HTML-001",
  "job_id": "JOB-HTML-001",
  "format": "html",
  "file_url": "https://example.com/receipt.html",
  "wait_ms": 2000
}
```

HTML 作业在 headless Chrome/Chromium/Edge 浏览器中渲染页面，导出为 PDF，然后通过常规 PDF 打印路径提交。`file_url` 必须是绝对 `http` 或 `https` URL。`wait_ms` 字段（默认 1000，最大 30000）控制渲染器在 PDF 导出前等待页面稳定的时间。

所有浏览器资源请求都经过过滤代理，该代理阻止非公共网络目标（回环、私有 IP、链路本地、多播）。这可防止不受信任的 HTML 页面发起 SSRF 攻击。

### Raw HTML 打印作业示例

```json
{
  "type": "print",
  "request_id": "REQ-RAW-HTML-001",
  "job_id": "JOB-RAW-HTML-001",
  "format": "raw-html",
  "html": "<h1>Shipping Label</h1><p>Order #12345</p>",
  "wait_ms": 500
}
```

`raw-html` 作业携带内联 HTML 而非 URL。使用相同的渲染流水线（通过过滤代理的 Chrome/Chromium）。内联 HTML 也通过代理加载，因此任何引用的资源（图片、样式表）必须位于公共网络目标上。

### 批量打印作业示例

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

批量作业可混合 PDF、图片、Office、HTML、raw-html 和 raw 格式。`batch_id` 和所有 `job_id` 必须唯一。批量大小受 `limits.max_batch_jobs`（默认 20）限制。批量执行仍使用同一个串行队列——并非并发打印。

### 作业状态推送

```json
{
  "type": "job_status",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "status": "queued",
  "message": "queued"
}
```

### 支持的格式

| 格式 | 输入 | 转换 |
|------|------|------|
| `pdf` | `file_url` 或 `data:application/pdf;base64,...` | 无 |
| `image` / `png` / `jpg` / `jpeg` | `file_url` | 图片 → PDF（适应纸张尺寸，203 DPI） |
| `docx` / `xlsx` / `pptx` | `file_url`（仅 HTTP/HTTPS） | Office → PDF，通过 LibreOffice（macOS/Linux）或 Windows COM |
| `html` | `file_url`（绝对 HTTP/HTTPS） | HTML → PDF，通过 headless Chrome/Chromium（带 SSRF 防护代理） |
| `raw-html` | `html`（内联字符串） | HTML → PDF，通过 headless Chrome/Chromium（带 SSRF 防护代理） |
| `raw` | `data_base64` | 无——字节原样提交 |

Office 任务仅支持 HTTP(S) `file_url`，不支持 data URL。HTML 任务支持可选的 `wait_ms` 字段（0–30000ms，默认 1000ms），控制渲染等待时间。

### 错误码

| 错误码 | 含义 |
|--------|------|
| `ORIGIN_NOT_ALLOWED` | WebSocket Origin 不在白名单中 |
| `INVALID_MESSAGE` | 消息格式错误 |
| `PRINTER_NOT_CONFIGURED` | 未设置默认打印机且未指定 |
| `PRINTER_NOT_FOUND` | 指定的打印机不存在 |
| `PAPER_NOT_CONFIGURED` | 未设置默认纸张且未指定 |
| `PAPER_NOT_FOUND` | 该打印机不支持此纸张尺寸 |
| `DOWNLOAD_FAILED` | 文件下载失败 |
| `FILE_TOO_LARGE` | 文件超过大小限制 |
| `UNSUPPORTED_FORMAT` | 不支持的格式 |
| `FORMAT_MISMATCH` | 声明的格式与文件内容不匹配 |
| `OFFICE_CONVERT_FAILED` | Office → PDF 转换失败 |
| `PRINT_FAILED` | 提交到操作系统打印失败 |
| `JOB_DUPLICATED` | `job_id` 已存在 |
| `BATCH_DUPLICATED` | `batch_id` 已存在 |
| `BATCH_TOO_LARGE` | 批量超过最大作业数限制 |
| `COPIES_OUT_OF_RANGE` | 份数超过最大限制 |
| `SERVICE_PORT_IN_USE` | 端口已被占用 |
| `INTERNAL_ERROR` | 未预期的内部错误 |

## 源码参考

| 领域 | 文件 |
|------|------|
| WS 处理器 + IP 中间件 | `crates/runtime/src/server.rs` |
| 消息类型 + 校验 + 错误码 | `crates/core/src/protocol.rs` |
| 每连接状态过滤 | `crates/runtime/src/server.rs` |
| 批量接受 + 去重 | `crates/runtime/src/queue.rs` |
| HTML 渲染 | `crates/runtime/src/html/` |

详细协议示例见 `docs/printbridge-technical.md`（WebSocket API 章节）。浏览器集成应使用 [print-bridge-sdk](https://github.com/vergil-lai/print-bridge-jssdk)，它封装了此协议。
