# PrintBridge — 快速开始

PrintBridge 是一个运行在用户本地电脑上的打印代理。它允许受信任的网页或远程业务服务器将 PDF 文件、图片、Office 文档、HTML 页面和原始打印机命令发送到本地系统打印队列——适用于标签、物流单据、收据、报表等需要可靠静默打印的业务场景。

它**不**替代打印机驱动程序，也**不**绕过操作系统打印队列。它接收任务、验证来源、下载或转换文件，然后将作业提交给本地操作系统。实际纸张输出仍由系统打印队列、打印机驱动程序和硬件处理。

## 技术栈

| 层级 | 技术 |
|------|------|
| 框架 | Tauri 2 |
| 前端 | Vue 3 + TypeScript |
| UI | shadcn-vue + Tailwind CSS |
| 构建 | Vite |
| 后端 | Rust + Axum + Tokio |
| 存储 | JSON 配置 + SQLite |
| Office 转换 | LibreOffice（macOS/Linux）/ Windows 原生 COM |
| HTML 渲染 | 通过 CDP 的 Headless Chrome/Chromium/Edge（带 SSRF 防护代理） |
| 平台打印 | SumatraPDF（Windows）/ CUPS `lp`（macOS/Linux） |

## 工作原理

```
网页 / 远程业务服务器
  │
  ├── WebSocket /ws（浏览器 → 代理，实时）
  │   或
  ├── HTTP 轮询（远程服务器 → 代理，周期性）
  │
  ▼
PrintBridge（验证来源 → 下载 → 转换 → 串行队列）
  │
  ▼
系统打印队列 → 打印机驱动 → 打印机
```

`submitted` / `success` 表示作业已被操作系统打印队列接受，**不**代表打印机已实际完成输出。

## 开发入门

```bash
pnpm install
pnpm tauri dev
```

Tauri 在 `http://localhost:1420/` 启动 Vite 开发服务器。本地打印服务监听 `0.0.0.0:17890`。

## 核心能力

- 系统托盘驻留，默认隐藏主窗口
- 跨平台：Windows、macOS、Linux
- 本地 HTTP/WebSocket 服务（默认端口 `17890`）
- 网站 Origin 白名单 + IP/CIDR 白名单（双层安全）
- 支持 PDF、PNG/JPEG 图片、Office（docx/xlsx/pptx）、HTML 页面和原始打印机命令（ESC/POS、TSPL、ZPL、EPL、PCL、PostScript）
- 串行打印队列——无并发打印机争用
- 适用于无人值守工作站的远程任务轮询
- CLI 运维模式（`print-bridge serve`、`print-bridge printer`、`print-bridge doctor` 等）
- 加密配置导入/导出，支持批量部署
- 应用内在线更新

## 文档导航

### [架构总览](architecture/overview.md)
Cargo workspace（core、runtime、cli、desktop、server）、Axum 服务器、打印队列 worker、远程轮询 worker 以及本地 IPC 命令系统的整体组织方式。

### [协议与 API](architecture/protocol.md)
WebSocket 打印协议（消息类型、作业生命周期、状态事件、错误码），包括 HTML、raw-html、PDF、图片、Office 和 raw 格式。集成必备。

### [打印流水线](workflows/printing-pipeline.md)
作业如何在串行队列中流转：HTML 渲染（Chrome/Chromium）、下载 → 格式检测 → 转换（Office/图片 → PDF）→ 平台特定的打印执行（Windows 使用 SumatraPDF，macOS/Linux 使用 CUPS `lp`）。

### [远程任务轮询](workflows/remote-polling.md)
服务器发起的任务投递：轮询/报告 HTTP 协议、基于 SQLite 的去重、基于 outbox 的状态报告（带指数退避）以及配置错误处理。

### [安全模型](domain/security.md)
双层白名单架构（IP + Origin）、配置传输加密（Argon2id + AES-256-GCM）以及安全最佳实践。

### [配置](domain/configuration.md)
完整配置结构（`service`、`security`、`printing`、`limits`、`app`、`remote`）、数据目录路径、环境变量和字段参考。

### [运维与部署](operations/deployment.md)
CLI 命令参考、headless `serve` 模式、`doctor` 诊断、systemd 打包、故障排查以及平台特定的部署指南。

### [源码地图](source-map.md)
开发者代码库指南：逐文件职责说明、注意事项和关键设计决策。

## 集成方式

| 使用方 | 协议 | 参考 |
|--------|------|------|
| 浏览器网页 | WebSocket `/ws` | [协议与 API](architecture/protocol.md) — 或使用 [print-bridge-sdk](https://github.com/vergil-lai/print-bridge-jssdk) |
| 业务服务器 | HTTP 轮询 | [远程任务轮询](workflows/remote-polling.md) — 服务器示例见 `examples/remote-task/` |
| ERP/批量部署 | 加密配置 | [安全模型](domain/security.md) — 跨语言生成器示例见 `examples/config-transfer/` |
| 设置界面/诊断 | HTTP REST | [协议与 API](architecture/protocol.md#http-api) |

## 主要源文件

最重要的入口文件快速参考：

| 领域 | 路径 |
|------|------|
| 应用入口（GUI） | `apps/desktop/src-tauri/src/lib.rs` |
| Headless 入口 | `apps/server/src/main.rs` |
| HTTP/WS 服务器 | `crates/runtime/src/server.rs` |
| WebSocket 协议 | `crates/core/src/protocol.rs` |
| 打印队列 + 流水线 | `crates/runtime/src/queue.rs` |
| HTML 渲染 | `crates/runtime/src/html/` |
| Office 转换 | `crates/runtime/src/office.rs` |
| 配置 | `crates/core/src/config.rs` |
| CLI 框架 | `crates/cli/src/` |
| 前端 | `apps/desktop/src/App.vue` |

## 已有文档

| 文档 | 语言 |
|------|------|
| `README.md` | 中文 |
| `README_en.md` | 英文 |
| `docs/printbridge-technical.md` | 中文（详细的协议、API、配置、部署） |
| `docs/printbridge-technical_en.md` | 英文 |
| `AGENTS.md` | AI agent 开发指南（中文） |
