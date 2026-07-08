# PrintBridge 技术说明

[English](./printbridge-technical_en.md)

## 技术栈

| 层级        | 技术                                                                                 |
| ----------- | ------------------------------------------------------------------------------------ |
| 框架        | [Tauri 2](https://v2.tauri.app/)                                                     |
| 前端        | [Vue 3](https://vuejs.org/) + [TypeScript](https://www.typescriptlang.org/)          |
| UI          | [shadcn-vue](https://www.shadcn-vue.com/) + [Tailwind CSS](https://tailwindcss.com/) |
| 构建        | [Vite](https://vite.dev/)                                                            |
| 后端        | Rust + [Axum](https://docs.rs/axum/latest/axum/) + [Tokio](https://tokio.rs/)        |
| 存储        | JSON配置文件 + [SQLite](https://www.sqlite.org/)                                     |
| Office 转换 | [office2pdf](https://docs.rs/office2pdf/latest/office2pdf/)                          |
| 平台打印    | [SumatraPDF](https://www.sumatrapdfreader.org/)(Windows) / CUPS `lp` (macOS/Linux)   |

## 产品边界

PrintBridge 是本机打印 Agent，不是打印机驱动，也不替代系统打印队列。

- 配置中的 `service.host` 保持为 `127.0.0.1` 兼容字段；当前服务实际绑定 `0.0.0.0:{port}`
- 浏览器侧主要通过 WebSocket `/ws` 下发任务
- 安全模型是 Origin 白名单
- 打印队列串行执行
- `submitted` / `success` 表示已提交到系统打印队列，不表示打印机已经真实出纸
- Windows 使用随包资源中的 SumatraPDF
- macOS 和 Linux 使用系统 CUPS 命令行工具

## 支持范围

当前版本适合以下文件类型：

- PDF
- PNG/JPEG 图片，任务格式可传 `image`，本地服务会按文件内容识别
- docx/xlsx/pptx Office 文件，PrintBridge 会先使用内置转换器生成 PDF，再提交到系统打印队列。打印效果以转换后的 PDF 为准，不保证与 Microsoft Office、WPS 或 LibreOffice 的渲染结果完全一致。
- 原始打印指令（Raw Commands），任务格式传 `raw`，内容使用 `data_base64`

raw 模式可以承载调用方已经生成好的 ESC/POS、TSPL、TSPL2、ZPL、EPL、PCL、PostScript 等设备指令。PrintBridge 只负责把 bytes 原样提交到系统打印队列，不解析这些指令，也不生成标签、小票或 RFID 指令。

原始打印指令任务不支持 `file_url`、`paper`、`copies`。如果业务需要多份 raw 输出，应由业务系统生成多份 raw 指令或发送多个 job。

## 开发

安装依赖：

```bash
pnpm install
```

启动桌面开发模式：

```bash
pnpm tauri dev
```

Tauri 会同时启动 Vite 开发服务：

```text
http://localhost:1420/
```

本地服务默认监听：

```text
0.0.0.0:17890
```

同机 Web 页面通常连接 `127.0.0.1`。局域网内其他设备如果需要连接这台电脑上的 PrintBridge 服务，可以使用这台电脑的局域网 IP，例如 `192.168.1.23`。

## 使用前配置

首次运行后，需要在 PrintBridge 设置界面完成：

1. 选择默认打印机
2. 选择或填写默认纸张
3. 把业务系统的 Origin 加入白名单，例如 `https://example.com`
4. 如果需要远程任务轮询，在“远程”选项卡填写任务 URL 并打开开关

Origin 必须精确匹配浏览器 WebSocket 握手中携带的 `Origin`，包括协议、域名和端口。示例：

```text
http://localhost:5173
https://example.com
```

白名单只校验发起连接的网页来源，不校验被打印文件 URL 的域名。

## CLI 运维入口

PrintBridge 提供 `print-bridge` CLI，用于在不打开 GUI 的情况下查看和修改本机配置。CLI 在 Tauri GUI 启动前执行，不依赖 Tauri CLI 插件，也不要求本地 Agent 服务正在运行。

```bash
print-bridge printer
print-bridge printer "Printer Name"
print-bridge printer set-default "Printer Name"

print-bridge paper
print-bridge paper set 60 40

print-bridge origin
print-bridge origin add "https://example.com"
print-bridge origin delete "https://example.com"

print-bridge remote
print-bridge remote enable
print-bridge remote disable
print-bridge remote set-url "https://example.com/print-task"
print-bridge remote set-token "token"
print-bridge remote set-device-id "factory-pi-01"
print-bridge remote set-device-name "packing-station-01"
print-bridge remote set-interval 10

print-bridge task
print-bridge task "JOB-001"
print-bridge task clear
```

CLI 直接读写与 GUI 相同的 `config.json`，并读取本地 `task_history.sqlite3`。它适合 macOS/Linux 的无头主机、server 版系统或树莓派等没有常驻 GUI 的部署环境。

## 配置导出与导入

设置界面支持把部分配置导出为加密 JSON 文件，也可以从加密 JSON 文件导入配置。导出文件默认名为：

```text
printbridge-config.json
```

导出时可以勾选以下配置项，默认全部选中：

- 本地端口
- Origin 白名单列表
- 远程任务开关
- 远程任务 URL
- 远程任务 Authorization Token
- 轮询时间
- 上报重试次数

导出时需要填写密码，密码可以留空；留空时仍然会按同一套加密流程生成文件。如果勾选了 Authorization Token 且密码为空，界面会要求二次确认。

导入时需要选择配置文件并输入导出时使用的密码。导入会先展示预览，确认后只覆盖文件中包含的配置项；文件中没有包含的配置会保留现有值。

Authorization Token 有额外保护规则：导入文件中 token 字段缺失、为 `null` 或为空字符串时，都会保留当前 token；只有非空字符串才会覆盖当前 token。

加密文件是普通 JSON 外壳，内部配置 payload 使用 Argon2id v1.3 和 AES-256-GCM 加密：

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

解密后的 payload 格式为：

```json
{
  "format": "printbridge-config",
  "version": 1,
  "config": {
    "service": {
      "port": 17890
    }
  }
}
```

ERP 或其他系统如果需要生成可导入配置，可以参考 `examples/config-transfer/` 下的 PHP、Go 和 Node 实现。桌面端项目目录提供统一验证命令：

```bash
pnpm verify:config-transfer-examples
```

## HTTP API

HTTP API 主要供桌面设置界面和诊断使用。

```text
GET  /health
GET  /printers
GET  /printers/{printer_name}/papers
GET  /config
POST /config
GET  /logs
POST /print/test
GET  /ws
```

`POST /print/test` 会使用当前默认打印机和默认纸张提交一张校准测试页，成功时返回 `202 Accepted`。它只允许桌面设置界面的 Origin 调用。

配置示例：

```json
{
  "service": {
    "host": "127.0.0.1",
    "port": 17890
  },
  "security": {
    "allowed_origins": []
  },
  "printing": {
    "default_printer": null,
    "default_paper": null,
    "default_copies": 1
  },
  "limits": {
    "max_file_size_mb": 20,
    "max_batch_jobs": 20,
    "max_copies": 100,
    "download_timeout_seconds": 30
  },
  "app": {
    "autostart": false
  },
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

`service.host` 是兼容字段，保存时保持为 `127.0.0.1`；当前本地服务实际固定监听所有网卡。

## WebSocket API

连接地址：

```text
ws://127.0.0.1:17890/ws
```

浏览器连接时，本地服务会在握手阶段校验 `Origin`。如果不在白名单中，连接会被拒绝。

### 心跳

请求：

```json
{
  "type": "ping",
  "time": 1780000000000
}
```

响应：

```json
{
  "type": "pong",
  "time": 1780000000000
}
```

### 单个打印任务

```json
{
  "type": "print",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "format": "pdf",
  "printer_name": "Office Printer",
  "file_url": "https://example.com/label.pdf",
  "copies": 1,
  "paper": {
    "width_mm": 60,
    "height_mm": 40
  }
}
```

`printer_name` 可以省略。省略时使用设置里的默认打印机。`paper` 可以省略。省略时使用设置里的默认纸张。

Office 任务：

```json
{
  "type": "print",
  "request_id": "REQ-OFFICE-001",
  "job_id": "JOB-OFFICE-001",
  "format": "docx",
  "file_url": "https://example.com/report.docx",
  "copies": 1,
  "paper": {
    "width_mm": 210,
    "height_mm": 297
  }
}
```

Office 文件支持 `docx`、`xlsx` 和 `pptx`，本地服务会先转换为临时 PDF，再进入 PDF 打印链路。Office 任务只支持 HTTP(S) `file_url`，不支持 data URL。

raw 任务：

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

raw 任务不支持 `file_url`、`paper`、`copies`。TSPL/TSPL2 这类标签指令中的纸张、间隙、份数、文字、条码、RFID 等参数需要由业务系统写入 raw 指令内容。

### 批量打印任务

```json
{
  "type": "print_batch",
  "request_id": "REQ-002",
  "batch_id": "BATCH-001",
  "jobs": [
    {
      "job_id": "A-001",
      "format": "image",
      "file_url": "https://example.com/a.png",
      "copies": 1
    },
    {
      "job_id": "B-001",
      "format": "raw",
      "printer_name": "TSC TE244",
      "data_base64": "XlhB..."
    }
  ]
}
```

批量任务只表示一次接收多个 job，执行时仍然使用同一个串行队列。批量任务可以混合 PDF、image、Office 和 raw。

### 任务状态

任务被接收后，本地服务会返回或推送 `job_status`：

```json
{
  "type": "job_status",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "status": "queued",
  "message": "queued"
}
```

状态值包括：

```text
queued
downloading
printing
submitted
completed
failed
unknown
cancelled
```

## 远程任务轮询

远程任务轮询适合业务服务器统一维护打印任务，而 PrintBridge 只负责在用户本机按配置拉取并打印的场景。

开启后，PrintBridge 会使用同一个 `remote.endpoint_url` 执行两种请求：

```text
GET  {endpoint_url}  拉取任务
POST {endpoint_url}  上报任务状态
```

如果配置了 `bearer_token`，请求会携带：

```text
Authorization: Bearer <bearer_token>
```

如果配置了设备标识，也会携带对应请求头：

```text
X-PrintBridge-Device-Id: <device_id>
X-PrintBridge-Device-Name: <device_name>
```

`device_id` 和 `device_name` 都是可选字段；只填写其中一个时，只发送对应的请求头和上报字段。界面里的“随机生成”按钮会生成 UUID v4 作为 `device_id`。

### 拉取任务

`GET` 响应可以为空、`null`、单个任务对象，或任务对象数组。单个任务格式：

```json
{
  "type": "print",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "format": "pdf",
  "file_url": "https://example.com/label.pdf",
  "copies": 1
}
```

raw 任务格式：

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

批量任务格式：

```json
{
  "type": "print_batch",
  "request_id": "REQ-002",
  "batch_id": "BATCH-001",
  "jobs": [
    {
      "job_id": "A-001",
      "format": "image",
      "file_url": "https://example.com/a.png",
      "copies": 1
    },
    {
      "job_id": "B-001",
      "format": "image",
      "file_url": "https://example.com/b.jpg",
      "copies": 2
    }
  ]
}
```

远程任务使用和 WebSocket 相同的打印字段。`job_id` 是远程去重键；已经记录过的 `job_id` 会被忽略，不会重复进入打印队列。

### 状态上报

PrintBridge 只向远程服务器上报三个状态：

```text
accepted
success
failed
```

本地队列状态会按以下规则映射：

```text
queued    -> accepted
submitted -> success
failed    -> failed
cancelled -> failed
```

`downloading`、`printing`、`completed` 和 `unknown` 只保留在本地日志、任务历史和 WebSocket 状态中，不上报给远程服务器。

状态上报请求体示例：

```json
{
  "event": "status",
  "event_id": "8c3f0f3a-0f6c-44c1-9e8e-1f0a60f5c813",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "status": "success",
  "message": "submitted to system print queue",
  "occurred_at": "2026-07-06T10:00:00Z",
  "device_id": "f77160d2-fa59-4ddb-93d9-205cd2dec3ac",
  "device_name": "packing-station-01"
}
```

`event_id` 由 PrintBridge 本地生成 UUID v4，并持久化到 SQLite。远程服务器可以用它做状态上报的幂等键。

PrintBridge 只把 HTTP `2xx` 视为上报成功。网络错误或非 `2xx` 响应会按 `max_report_retries` 重试，默认最多 10 次。`401`、`403` 和 `404` 会被视为配置类错误，远程轮询和状态上报会暂停，等待用户修改配置后恢复。

保存远程配置或点击“测试连接”时，PrintBridge 会先用同一个 URL 测试 `GET` 和 `POST`。测试请求会携带：

```text
X-PrintBridge-Test: true
```

## 开发验证

前端检查：

```bash
pnpm typecheck
pnpm build
```

Rust 检查：

```bash
cd src-tauri
cargo fmt --check
cargo check
cargo clippy --tests -- -D warnings
cargo test
```

部分 Rust 测试会绑定本地 TCP 端口。如果沙箱或安全软件拦截本地网络，先在普通终端重跑后再判断是否是代码问题。

## 平台说明

Windows 打印依赖随包资源中的 SumatraPDF：

```text
src-tauri/resources/windows/SumatraPDF.exe
```

当前资源来自 SumatraPDF 3.6.1 64-bit portable：

```text
ZIP SHA-256: 98b33a518d42986856d225064b0cd2d3643ecf78cbf84ab873d26cc51877a544
EXE SHA-256: 719f689b34f47be8ca105ce8484948474dafde0e106bab599e4a89326070c3d0
```

macOS 和 Linux 打印依赖系统 CUPS 命令行工具：`lpstat`、`lpoptions` 和 `lp`。PrintBridge 不会自动安装 CUPS 或打印机驱动；如果系统缺少这些命令，CLI 和 Agent 会返回明确错误，提示先安装或启用 CUPS。
