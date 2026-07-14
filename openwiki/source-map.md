# 源码地图

开发者代码库指南：文件职责、修改各区域时的注意事项，以及关键设计决策。

## 项目结构

```
PrintBridge/                       # Cargo workspace（5 个成员）
├── crates/
│   ├── core/                      # 领域模型、协议、配置——无框架依赖
│   │   └── src/
│   │       ├── config.rs          # AgentConfig + 各配置段，加载/保存，默认值
│   │       ├── protocol.rs        # ClientMessage/ServerMessage，校验，ErrorCode
│   │       ├── ip_whitelist.rs    # IP/CIDR 校验 + 运行时检查
│   │       ├── printing.rs        # PrintBackend trait，PrinterInfo/PaperInfo 类型
│   │       ├── queue.rs           # QueueState 类型，QueueError
│   │       ├── remote_protocol.rs # RemoteTask 类型 + 解析器
│   │       └── activity.rs        # TaskHistoryJob，TaskHistoryEvent，TaskLogEntry
│   ├── runtime/                   # AgentRuntime，平台适配器，后台任务
│   │   └── src/
│   │       ├── agent.rs           # AgentRuntime + AgentHandle 生命周期
│   │       ├── builder.rs         # RuntimeBuilder（路径 + 适配器 → AgentRuntime）
│   │       ├── state.rs           # AgentState（共享配置、队列、stores、适配器）
│   │       ├── server.rs          # Axum 路由（仅暴露 /ws）
│   │       ├── queue.rs           # 串行打印队列 + 作业处理流水线
│   │       ├── command_executor.rs # RuntimeCommandExecutor（离线 Command 执行器）
│   │       ├── doctor.rs          # Doctor 诊断检查
│   │       ├── html/              # HTML → PDF 渲染（通过 CDP 调用 Chrome/Chromium）
│   │       ├── printing/          # 平台打印后端（CUPS、Windows）
│   │       ├── office/            # Office → PDF 转换（LibreOffice / Windows COM）
│   │       ├── document.rs        # Magic byte 检测，图片→PDF
│   │       ├── download.rs        # HTTP/HTTPS/data URL 下载到临时文件
│   │       ├── ipc/               # 本地 IPC（Unix socket / Windows 命名管道）
│   │       ├── remote_*.rs        # 远程任务轮询子系统
│   │       ├── task_history.rs    # SQLite 任务历史存储
│   │       ├── logs.rs            # 内存日志环形缓冲区
│   │       ├── test_print.rs      # 校准测试页
│   │       └── agent_guard.rs     # 实例独占（端口探测）
│   └── cli/                       # Command 枚举，CommandService，CLI 解析器，IPC client
│       └── src/
│           ├── command.rs         # Command 枚举 + 策略（在线/离线偏好）
│           ├── service.rs         # CommandService（在线/离线分发）
│           ├── parser.rs          # Clap 命令结构 + CLI 处理器
│           ├── client.rs          # LocalClientExecutor（IPC client）
│           ├── output.rs          # CommandResult，CommandError，DoctorReport 类型
│           ├── policy.rs          # CommandPolicy 枚举
│           ├── product.rs         # ProductCommandAdapter（自启动、语言）
│           ├── interaction.rs     # 终端交互提示
│           └── config_transfer.rs # 加密配置导入/导出
├── apps/
│   ├── desktop/                   # Vue 3 前端 + Tauri 后端
│   │   ├── src/                   # 前端（Vue 3 + TypeScript）
│   │   │   ├── App.vue            # 单体设置界面
│   │   │   ├── api.ts             # Tauri invoke + HTTP 封装
│   │   │   ├── types.ts           # 共享 TypeScript 类型
│   │   │   ├── i18n.ts            # vue-i18n（zh-CN、en）
│   │   │   └── updater.ts         # Tauri 更新器封装
│   │   └── src-tauri/             # 桌面 Rust 后端
│   │       ├── src/
│   │       │   ├── main.rs        # 二进制入口
│   │       │   ├── lib.rs         # Tauri 应用初始化 + runtime 装配
│   │       │   ├── cli.rs         # 桌面 CLI 分发
│   │       │   ├── product_cli.rs # 桌面产品适配器（自启动、语言）
│   │       │   ├── commands.rs    # Tauri 命令（配置、日志、测试打印）
│   │       │   └── tray.rs        # 系统托盘
│   │       ├── resources/         # SumatraPDF.exe（Windows）
│   │       └── tests/             # 集成测试
│   └── server/                    # Linux headless 二进制
│       ├── src/
│       │   ├── main.rs            # 二进制入口：serve / 共享 CLI 分发
│       │   ├── dependencies.rs    # 预检（CUPS、LibreOffice、Chrome）
│       │   ├── parser.rs          # 服务器 CLI 参数
│       │   ├── paths.rs           # 系统路径（/etc、/var/lib、/run）
│       │   ├── readiness.rs       # systemd READY/STOPPING 通知
│       │   └── signals.rs         # SIGTERM/SIGINT 处理
│       ├── packaging/             # deb/rpm/systemd unit 文件
│       └── tests/
├── examples/                      # 服务端参考实现
├── scripts/                       # 构建/发布工具
├── docs/                          # 技术文档
└── .github/workflows/             # CI（发布 + OpenWiki）
```

## Core Crate（`crates/core`）

无框架依赖的领域模型和协议类型。不依赖 Tauri、Axum、Clap 或 SQLite。

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `protocol.rs` | `ClientMessage`/`ServerMessage` 类型，`JobStatus`，`ErrorCode`，`PrintJobInput` 校验，`SupportedFormat` 枚举。 | `validate_for_acceptance` 是作业接受的守门人。错误码是协议稳定的。现已包含 `Html` 和 `RawHtml` 格式 + `html`/`wait_ms` 字段。 |
| `config.rs` | `AgentConfig` + 各配置段结构体，默认值，`load`/`save`，`normalized`，数据目录解析。 | `normalized()` 强制 `host=127.0.0.1`。配置类型必须与前端 `types.ts` 匹配。 |
| `ip_whitelist.rs` | IP/CIDR 校验，`is_client_ip_allowed`，拒绝允许所有条目。 | `127.0.0.1` 被强制且不可变。不信任代理头。 |
| `printing.rs` | `PrintBackend` trait，`PrinterInfo`，`PaperInfo`，`PrintOptions`，`PrintSubmission`，`PrintTrackingOutcome`。 | 这些是无框架类型——平台实现位于 `crates/runtime`。 |
| `queue.rs` | 队列状态类型和 `QueueError`。 | |
| `remote_protocol.rs` | `RemoteTask` 枚举 + 灵活解析器。 | 解析器接受单个对象、数组、null 或空字符串。 |
| `activity.rs` | `TaskHistoryJob`，`TaskHistoryEvent`，`TaskLogEntry`——共享活动类型。 | |

## Runtime Crate（`crates/runtime`）

AgentRuntime 生命周期、平台适配器和所有后台任务。

### Agent 生命周期

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `agent.rs` | `AgentRuntime`（启动前），`AgentHandle`（运行中）。`start()` 绑定监听器，启动 server/queue/remote/IPC 任务。 | 四个任务共享一个 `CancellationToken`。`shutdown()` 等待完成并删除 IPC socket。 |
| `builder.rs` | `RuntimeBuilder`——路径 + 可选打印后端和 HTML 渲染器 → `AgentRuntime`。`RuntimePaths`（配置、数据、运行时目录）。 | 默认：`printing::default_backend()` 和 `BrowserHtmlRenderer`。 |
| `state.rs` | `AgentState`——共享容器：配置、队列、stores、打印后端、HTML 渲染器、IPC 执行器。 | 通过 `Arc` 低成本克隆。`status_events` 广播容量为 128。 |
| `command_executor.rs` | `RuntimeCommandExecutor`——`Command` 的离线执行器（Agent 未运行时使用）。 | |
| `doctor.rs` | `run_doctor`——只读诊断检查（配置有效性、数据目录、端口、打印机、浏览器、Office、systemd、远程）。 | 检查浏览器（Chrome/Chromium）和 LibreOffice 可用性。 |

### 服务器与 IPC

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `server.rs` | Axum 路由，仅暴露 `/ws`。IP 白名单中间件，WebSocket 处理器，Origin 校验。 | 无 HTTP REST 端点——配置/日志/打印机/测试打印通过 Tauri 命令或 CLI/IPC 处理。 |
| `ipc/mod.rs` | 本地 IPC 的平台分发（Unix 上 Unix socket，Windows 上命名管道）。 | 4 字节大端长度前缀，JSON 信封，协议版本 1，最大 8 MiB 帧。Socket 位于 `agent.sock`，权限 `0660`。 |
| `ipc/unix.rs` | Unix 域 socket 实现。 | |
| `ipc/windows.rs` | 命名管道实现。 | |

### 打印队列与流水线

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `queue.rs` | `QueueState`（FIFO + 去重），`run_worker`（串行循环），`process_job_inner`（下载 → 转换 → 打印）。`html`、`raw-html` 和 `raw` 作业有独立路径。 | 单 worker = 严格串行执行。`print_html_job` 渲染为临时 PDF 后复用 PDF 提交路径。临时文件清理至关重要。 |
| `printing/mod.rs` | 平台分发（`default_backend`），`sumatra_print_settings`。 | 后端在编译时选择（`#[cfg(target_os = ...)]`）。 |
| `printing/cups.rs` | macOS/Linux 后端：`lp`、`lpstat`、`lpoptions`。通过 `lpstat -W completed` 跟踪作业。 | 从 `lp` 输出解析作业 ID 较脆弱。 |
| `printing/windows.rs` | Windows 后端：PDF 使用 SumatraPDF CLI，raw 使用 Win32 Spooler API。 | 无作业跟踪（`tracking_supported: false`）。 |
| `document.rs` | Magic byte 检测（PDF/PNG/JPEG），通过 `printpdf` 进行图片→PDF 转换。 | 标签打印机假设 203 DPI。 |
| `office.rs` + `office/` | Office 格式检测 + 转换。macOS/Linux：LibreOffice（`libreoffice.rs`）。Windows：原生 COM（`windows.rs`）。 | 转换保真度取决于 LibreOffice 渲染——不保证与 MS Office 完全一致。120 秒转换超时。 |
| `download.rs` | `download_to_temp`：HTTP/HTTPS 流式 + data URL。双层大小限制。超时。 | Content-Length 检查 + 流式字节计数。出错时清理部分文件。 |

### HTML 渲染（`html/`）

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `mod.rs` | `HtmlRenderer` trait，`HtmlRenderRequest`，`HtmlSource`（Url 或 Inline），`HtmlRenderError`。 | Trait 是异步的（`Pin<Box<dyn Future>>`），允许测试注入。 |
| `browser.rs` | `BrowserHtmlRenderer`——查找 Chrome/Chromium/Edge，带代理启动，通过 CDP 渲染为 PDF。 | 通过过滤代理渲染以防止 SSRF。60 秒渲染超时，每个 CDP 操作 10 秒。按平台特定路径发现浏览器。 |
| `resource_policy.rs` | `ResourcePolicy`——阻止非公共 IP（回环、私有、链路本地、多播）。连接前解析 DNS。 | 防止对内部服务的 SSRF。阻止 `file:`、`data:`、`localhost`、`127.0.0.1`、`10.x`、`169.254.x` 等。 |
| `proxy.rs` | `FilteringProxy`——本地 HTTP 代理，拦截所有浏览器资源请求并应用 `ResourcePolicy`。 | 所有 Chrome 流量通过 `--proxy-server` 强制经过代理。被拒绝的资源以 `BlockedResource` 中止渲染。 |

### 远程轮询

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `remote_client.rs` | HTTP client：`fetch_tasks`、`report_status`、`test_connection`。 | `is_configuration_status()` 将 401/403/404 视为配置错误 → 退避。 |
| `remote_store.rs` | SQLite：`remote_jobs`（去重）+ `remote_status_events`（outbox）。 | `INSERT OR IGNORE` 保证幂等性。 |
| `remote_worker.rs` | 无限循环：轮询 → 入队 → 报告。配置错误时暂停。 | 通过 `retry_at_string` 实现报告退避。 |

## CLI Crate（`crates/cli`）

无框架依赖的命令类型和 CLI 解析器，由桌面和 headless 产品共享。

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `command.rs` | `Command` 枚举（GetConfig、SaveConfig、ListPrinters、Doctor、Status、ExportConfig、ImportConfig 等）。每个命令有一个 `CommandPolicy`。 | 添加命令需要更新 `policy()` 和执行器实现。 |
| `service.rs` | `CommandService`——根据策略将 `Command` 分发到在线（通过 IPC 的 Agent）或离线执行器。 | 仅 `NotRunning` 错误会触发 `OnlinePreferred` 命令的离线回退。 |
| `parser.rs` | Clap 命令结构（status、config、printer、paper、origin、remote、task、logs、test-remote、test-print、autostart、app、service、ip、doctor）。 | CLI 子命令必须与 `Command` 变体匹配。 |
| `client.rs` | `LocalClientExecutor`——通过 IPC 向运行中的 Agent 发送命令。 | |
| `output.rs` | `CommandResult`、`CommandError`、`AgentStatus`、`DoctorReport`/`DoctorCheck`/`DoctorSummary`、`ProductKind`。 | 错误类型映射到稳定的退出码。 |
| `policy.rs` | `CommandPolicy` 枚举（`OnlineOnly`、`OnlinePreferred`、`OfflineAllowed`）。 | |
| `product.rs` | `ProductCommandAdapter` trait（自启动、语言）。桌面适配器已启用；headless 使用 `UnsupportedProductCommandAdapter`。 | |
| `config_transfer.rs` | 加密导出/导入：Argon2id + AES-256-GCM，字段选择，合并，预览 diff。 | Bearer token 有特殊保护（空/null → 保留原值）。 |

### 历史与诊断

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `task_history.rs` | SQLite：`task_history_jobs`（聚合）+ `task_history_events`（仅追加）。 | `finished_at` 仅在终态时设置。 |
| `logs.rs` | `LogStore`——内存环形缓冲区（500 条）。不持久化。 | 满时丢弃最旧条目。 |
| `test_print.rs` | 校准测试页生成。 | 使用默认打印机 + 纸张。 |
| `agent_guard.rs` | TCP 端口探测以检测运行中的实例。 | GUI 中止启动；headless 返回 `RuntimeError::AlreadyRunning`。 |

## App Crate

### 桌面（`apps/desktop`）

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `src-tauri/src/main.rs` | 二进制入口。 | |
| `src-tauri/src/lib.rs` | Tauri 应用初始化：runtime builder、托盘、命令服务装配（在线 + 离线执行器）、窗口关闭拦截。 | 重新导出 `print_bridge_runtime` 和 `print_bridge_core` 模块。窗口关闭时隐藏到托盘。 |
| `src-tauri/src/cli.rs` | 桌面 CLI 分发。 | |
| `src-tauri/src/product_cli.rs` | `DesktopProductCommandAdapter`——通过 `auto_launch` 实现自启动、语言设置。 | macOS 使用 `LaunchAgent=false`（改用 `.plist`）。Linux 解析 `APPIMAGE` 路径。 |
| `src-tauri/src/commands.rs` | Tauri 命令：配置 CRUD、导出/导入、测试连接、日志、任务历史、测试打印。 | `print_test` 仅限设置界面 origin。 |
| `src-tauri/src/tray.rs` | 系统托盘：打开设置、测试打印、查看日志、重启、开机启动、退出。已本地化。 | `toggle_autostart` 持久化到配置。 |

### 服务器（`apps/server`）

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `src/main.rs` | Headless 二进制入口。无参数 → 帮助。`serve` → `RuntimeBuilder` + `AgentRuntime::start`。其他参数 → 通过 `run_cli_from` 共享 CLI。 | 使用 `UnsupportedProductCommandAdapter`（自启动由 systemd 管理，语言固定）。 |
| `src/dependencies.rs` | `preflight()`——在 serve 启动前校验 `lp`/`lpstat`/`lpoptions`（CUPS）、`soffice`/`libreoffice` 和 Chrome/Chromium 是否在 PATH 上。 | |
| `src/paths.rs` | `system_paths()`——解析 `/etc/print-bridge`、`/var/lib/print-bridge`、`/run/print-bridge`。 | |
| `src/readiness.rs` | 通过 `sd_notify` 向 systemd 发送 `READY=1` / `STOPPING=1`。 | |
| `src/signals.rs` | 处理 `SIGTERM` / `SIGINT` 以实现优雅关闭。 | |

## 前端（`apps/desktop/src`）

| 文件 | 职责 | 注意事项 |
|------|------|---------|
| `App.vue`（约 71 KB） | 包含所有 UI 的单体 SFC：设置、远程、网站白名单、IP 白名单、任务、关于标签页。无路由。 | 单文件包含所有 UI 逻辑。改变端口的配置变更在生产环境中会触发应用重启。 |
| `api.ts` | 所有 API 调用通过 `invoke()`（Tauri 命令）。无直接 HTTP 请求。 | 必须与 Rust 类型匹配——修改需同时更新两端。 |
| `types.ts` | 共享 TypeScript 类型：`AgentConfig`、`PrinterInfo`、`PaperInfo`、`TaskHistoryJob`、`TaskHistoryEvent`。 | 必须与 Rust 结构体匹配。 |
| `i18n.ts` | vue-i18n v11（composition 模式）。zh-CN（默认）+ en。约 140 个键。 | 任务状态/来源标签是 App.vue 中的内联查找表。 |
| `updater.ts` | Tauri 更新器封装：检查、下载+安装、重启。 | 进度通过 `DownloadEvent` 回调。 |
| `main.ts` | 应用入口：创建 Vue 应用，安装 vue-i18n，挂载。 | |

## 辅助文件

| 路径 | 用途 |
|------|------|
| `apps/desktop/src-tauri/tauri.conf.json` | 应用配置：标识符 `com.vergil.printbridge`，窗口 960×680，更新器端点 |
| `apps/desktop/src-tauri/tauri.windows.conf.json` | Windows 覆盖配置：打包 `SumatraPDF.exe` 作为资源 |
| `apps/desktop/src-tauri/capabilities/default.json` | 主窗口权限：core、dialog、opener 默认 |
| `apps/desktop/src-tauri/capabilities/desktop.json` | 桌面权限：自启动、日志、更新器、进程、dialog、opener |
| `apps/server/packaging/deb/control` | deb 包元数据（`print-bridge-server`，依赖 systemd + cups-client + libreoffice） |
| `apps/server/packaging/rpm/print-bridge.spec` | RPM spec 文件 |
| `apps/server/packaging/systemd/print-bridge.service` | systemd unit：`Type=notify`，以 `printbridge` 用户运行，`ProtectSystem=strict` |
| `scripts/release.mjs` | 交互式发布：版本同步、标签检查、推送至 `release` 分支 |
| `scripts/build-server-packages.sh` | 为 headless 服务器构建 deb/rpm 包 |
| `scripts/patch-updater-json.mjs` | 发布后：重写 `latest.json` 资源 URL |
| `scripts/release-version.mjs` | 协调发布的版本校验 |
| `scripts/verify-config-transfer-examples.mjs` | 运行所有配置传输示例自测 |
| `.github/workflows/release.yml` | CI：构建桌面（macOS ARM+Intel、Linux、Windows）和服务器（deb/rpm）包 |
| `.github/workflows/sync-release-notes.yml` | 同步发布说明 |
| `.github/workflows/openwiki-update.yml` | 定时 OpenWiki 文档刷新 |
| `AGENTS.md` | AI agent 开发指南（中文） |
| `docs/printbridge-technical.md` | 详细技术文档（中文） |
| `docs/printbridge-technical_en.md` | 英文技术文档 |
