# 架构总览

PrintBridge 是一个 Cargo workspace，交付桌面 GUI 与 Linux headless 两种互斥产品。两个产品都使用 `print-bridge` 作为可执行文件名，但由不同软件包提供。

## Workspace

```text
crates/core       配置、协议、白名单、任务及打印模型
crates/runtime    AgentState、打印适配器、队列、worker、WebSocket 与本地 IPC
crates/cli        Command、CommandResult、CommandService、配置导入导出和 IPC client
apps/desktop      Vue 前端与 Tauri 后端
apps/server       Linux headless 入口与 deb/rpm/systemd packaging
```

依赖方向为 `core <- cli <- runtime <- desktop/server`。`core` 不依赖 Tauri、Axum、Clap 或 SQLite。

## 命令流

GUI、CLI 与 headless 本地管理统一构造强类型 `Command`，交给 `CommandService`。运行中的 Agent 通过 Unix socket（Windows 产品使用命名管道边界）处理命令；只有明确的 `NotRunning` 才允许离线回退，权限、协议或 runtime 错误不会被误认为 Agent 未启动。

本地 IPC 使用 4 字节大端长度前缀和 JSON envelope，协议版本为 1，最大帧 8 MiB。Unix socket 位于 runtime 目录的 `agent.sock`，权限固定为 `0660`。

## 网络接口

Agent 的 Axum router 只暴露 `/ws`。浏览器打印协议、心跳、打印机查询和任务状态都通过 WebSocket；配置、日志、导入导出和测试打印不提供 REST API。

## 产品形态

- Desktop：无参数启动 GUI；`serve` 是未知命令。
- Headless：无参数显示帮助；只有显式 `print-bridge serve` 才启动 Agent。
- Linux headless 软件包安装后由 systemd 自动启动，使用专用 `printbridge` 系统用户。

## 运行时生命周期

`RuntimeBuilder` 创建配置、数据和运行目录并组装 stores/adapters。`AgentRuntime::start` 绑定 WebSocket listener、启动 IPC、队列和远程 worker，返回 `AgentHandle`。所有任务共享 cancellation token；关闭时等待 worker、WebSocket 和 IPC 完成，并删除 Unix socket。
