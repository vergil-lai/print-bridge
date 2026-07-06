# PrintBridge

PrintBridge 是一个运行在用户电脑上的本地打印桥接程序。它让受信任的 Web 页面可以把 PDF 或图片打印任务发送到本机打印队列，也可以从远程业务服务器轮询打印任务并回报执行结果，用于标签、面单、小票等需要稳定静默打印的业务场景。

它不替代打印机驱动，也不绕过系统打印队列。PrintBridge 负责接收浏览器任务、校验来源、下载或转换文件，并把任务提交给本机操作系统。

## 功能

- 系统托盘常驻，默认隐藏主窗口
- 本地 HTTP/WebSocket 服务，默认端口 `17890`
- Origin 白名单，用于限制哪些 Web 页面可以连接本地服务
- 支持 `pdf` 和 `image` 打印任务，图片会先转换成 PDF
- 支持 HTTP(S) 文件 URL；PDF 额外支持 `data:application/pdf;base64,...`
- 串行打印队列，避免同一台打印机并发抢占
- 可选远程任务轮询，从业务服务器拉取打印任务并回报执行结果
- 打印机枚举、纸张枚举、配置持久化和最近任务日志
- Windows 使用随包资源中的 SumatraPDF 打印 PDF
- macOS 使用 CUPS `lp` 命令打印

## 软件截图

|                                                         |                                                         |
| ------------------------------------------------------- | ------------------------------------------------------- |
| ![PrintBridge 软件截图 1](screenshots/ScreenShot_1.png) | ![PrintBridge 软件截图 2](screenshots/ScreenShot_2.png) |
| ![PrintBridge 软件截图 3](screenshots/ScreenShot_3.png) | ![PrintBridge 软件截图 4](screenshots/ScreenShot_4.png) |

## 安装
在[Releases](https://github.com/vergil-lai/print-bridge/releases)下载最新版本。

## JS-SDK

PrintBridge 只包含本机桌面端程序和设置界面。浏览器侧接入请使用 [`print-bridge-sdk`](https://github.com/vergil-lai/print-bridge-jssdk)。

## 工作方式

```text
Web 页面
  |
  | WebSocket: ws://127.0.0.1:17890/ws
  v
PrintBridge 本地服务
  |
  | 下载 PDF / 图片，必要时转换 PDF
  v
系统打印队列
  |
  v
打印机驱动与打印机
```

`success` 表示任务已经成功提交到系统打印队列，不代表打印机已经完成出纸。

除了浏览器通过 WebSocket 主动下发任务，PrintBridge 也可以开启远程任务轮询模式：

```text
远程业务服务器
  ^
  | POST 状态上报
  |
  | GET 拉取任务
  v
PrintBridge 本地服务
  |
  | 下载 PDF / 图片，必要时转换 PDF
  v
系统打印队列
```

## 支持范围

当前版本适合以下文件类型：

- PDF
- PNG/JPEG 图片，任务格式可传 `image`，本地服务会按文件内容识别

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

`POST /print/test` 目前是预留接口，会返回 `NOT_IMPLEMENTED`。

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
success   -> success
failed    -> failed
cancelled -> failed
```

`downloading` 和 `printing` 只保留在本地日志中，不上报给远程服务器。

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
  "file_url": "https://example.com/label.pdf",
  "copies": 1,
  "paper": {
    "width_mm": 60,
    "height_mm": 40
  }
}
```

`paper` 可以省略。省略时使用设置里的默认纸张。

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
      "format": "image",
      "file_url": "https://example.com/b.jpg",
      "copies": 2
    }
  ]
}
```

批量任务只表示一次接收多个 job，执行时仍然使用同一个串行队列。

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
success
failed
cancelled
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

macOS 打印依赖系统 CUPS 工具。Linux 当前会明确返回不支持平台，而不是模拟成功。

## 安全边界

PrintBridge 运行在用户本机，能够访问本机打印机。部署时请至少做到：

- 只把可信业务系统加入 Origin 白名单
- 不要把本地服务端口暴露到不可信网络
- 在业务系统侧控制谁能发起打印、能打印哪些文件
- 不要把敏感文件 URL 暴露给不可信页面

## License

[MIT](./LICENSE)。

Windows 版本随包使用的 SumatraPDF 适用其自身许可证。详见 [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)。
