# 远程任务轮询

远程任务轮询让 PrintBridge 可以作为工作站、门店终端或仓库电脑上的无人值守代理运行。业务服务器维护待打印任务，本地代理周期性地轮询获取任务、提交到操作系统打印队列，并将执行状态报告回服务器。

适用于系统创建任务并期望特定终端自动打印的场景——生产标签、物流标签、拣货单、收据等。

## 轮询协议

代理对两个操作使用同一个 `remote.endpoint_url`：

```
GET  {endpoint_url}   → 获取待处理任务
POST {endpoint_url}   → 报告任务状态
```

### 认证头

```
Authorization: Bearer <bearer_token>        （如已配置）
X-PrintBridge-Device-Id: <device_id>        （如已配置）
X-PrintBridge-Device-Name: <device_name>    （如已配置）
```

三个字段均为可选且独立——仅发送已配置的字段。

### 任务获取响应

`GET` 响应是灵活的：
- `204 No Content` 或空响应体 → 无任务
- `null` → 无任务
- 单个任务对象 → 一个任务
- 任务对象数组 → 多个任务

任务使用与 WebSocket 作业相同的字段结构：

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

批量任务携带 `batch_id` 和 `jobs` 数组。`job_id` 是去重键——已见过的 `job_id` 的任务会被静默跳过。

### 状态报告请求体

```json
{
  "event": "status",
  "event_id": "8c3f0f3a-...",
  "request_id": "REQ-001",
  "job_id": "JOB-001",
  "status": "success",
  "message": "submitted to system print queue",
  "occurred_at": "2026-07-06T10:00:00Z",
  "device_id": "f77160d2-...",
  "device_name": "packing-station-01"
}
```

`event_id` 是本地生成的 UUID v4 并持久化到 SQLite——服务器可将其用作幂等键。

## 状态映射

PrintBridge 仅报告三种远程状态：

| 本地队列状态 | 报告的远程状态 |
|------------|--------------|
| `queued` | `accepted` |
| `submitted` | `success` |
| `failed` | `failed` |
| `cancelled` | `failed` |

`downloading`、`printing`、`completed` 和 `unknown` **不会**报告给远程服务器——它们仅保留在本地日志和任务历史中。

## Worker 循环（`remote_worker.rs`）

远程 worker 运行无限循环：

1. 读取配置；如果 `remote.enabled` 为 false → `await` `remote_notify`（配置变更时唤醒）
2. **轮询：** `fetch_tasks` → 校验 → 去重 → 入队到打印队列
3. **报告：** 从 SQLite outbox 投递待发送的状态事件
4. 如果发生配置错误（HTTP 401/403/404）→ `await` `remote_notify`（停止轰炸服务器）
5. 休眠 `poll_interval_seconds`（默认 10，最小 3）
6. 重复

## SQLite 持久化（`remote_store.rs`）

远程状态持久化在 `remote.sqlite3` 中，包含两个表：

### `remote_jobs`——去重表

| 列 | 用途 |
|------|------|
| `job_id`（主键） | 去重键 |
| `request_id`、`batch_id` | 原始请求跟踪 |
| `status` | 当前状态 |
| `first_seen_at`、`updated_at` | 时间戳 |

`record_job_if_new` 使用 `INSERT OR IGNORE`——仅在首次插入时返回 `true`。这提供了跨重启的去重：如果代理重启，已在队列中的任务不会被重新入队。

### `remote_status_events`——状态 Outbox

| 列 | 用途 |
|------|------|
| `event_id`（主键） | UUID v4，幂等键 |
| `job_id`、`status`、`message` | 要报告的状态 |
| `delivery_state` | `pending` / `delivered` / `abandoned` |
| `retry_count` | 失败时递增 |
| `next_retry_at` | 指数退避时间戳 |
| `last_error` | 最近一次失败的错误信息 |

`(job_id, status)` 上的唯一索引防止每个作业的重复状态报告。

### 带指数退避的投递

`report_pending_once` 获取最多 20 条待处理事件并逐条 POST：
- **成功**（HTTP 2xx）→ `mark_delivered`
- **失败**（非 2xx 或网络错误）→ `mark_delivery_failed` → 递增 `retry_count`，设置带指数退避的 `next_retry_at`，超过 `max_report_retries`（默认 10）后放弃
- **配置错误**（401/403/404）→ 立即传播，暂停轮询和报告直到配置修复

## 连接测试

设置界面的"测试连接"按钮和远程配置保存通过 `remote_client::test_connection` 触发测试：
1. 带 `X-PrintBridge-Test: true` 头向端点发送 `GET`
2. 带相同测试头发送 `POST`
3. 服务器应对测试请求返回 `204 No Content`

## 配置错误处理

HTTP 401、403 和 404 被视为**配置错误**——而非临时故障。遇到时，远程 worker 会同时暂停轮询和状态报告，并等待 `remote_notify`（在用户更新远程配置时触发）。这可防止反复轰炸配置错误或未授权的端点。

## 服务器实现示例

`examples/remote-task/` 中的参考实现演示了服务端 HTTP API：

| 语言 | 文件 | 示例任务 |
|------|------|---------|
| Node.js | `remote-task-server.mjs` | 单个 PDF（`JOB-NODE-PDF`） |
| PHP | `remote-task-server.php` | 单个图片（`JOB-PHP-IMAGE`） |
| Go | `remote-task-server.go` | PDF + 图片批量（`BATCH-GO-SAMPLE`） |

三者均：
- 监听 `127.0.0.1:18080`
- 使用 Bearer token `dev-token`
- 对 `X-PrintBridge-Test: true` 请求返回 `204`
- `GET` → 返回任务 JSON；`POST` → 记录状态，返回 `204`

## 源码参考

| 领域 | 文件 |
|------|------|
| HTTP client（获取 + 报告 + 测试） | `crates/runtime/src/remote_client.rs` |
| 任务/批量协议结构 | `crates/core/src/remote_protocol.rs` |
| SQLite 存储（去重 + outbox） | `crates/runtime/src/remote_store.rs` |
| Worker 循环（轮询 + 报告 + 退避） | `crates/runtime/src/remote_worker.rs` |
