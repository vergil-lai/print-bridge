# 打印流水线

打印流水线是核心处理路径：从作业接受到下载、格式转换、平台特定打印和状态跟踪。所有作业——无论来自 WebSocket 还是远程轮询——都经过同一个串行 FIFO 队列。

## 串行队列架构

打印队列是在 `QueueState`（`queue.rs`）中用 `VecDeque<QueuedJob>` 实现的严格 FIFO：

```rust
struct QueueState {
    pending: VecDeque<QueuedJob>,
    seen_job_ids: HashSet<String>,
    seen_batch_ids: HashSet<String>,
}
```

**去重：** 每个 `job_id` 在接受前都会与 `seen_job_ids` 比对。重复的将被拒绝并返回 `JOB_DUPLICATED`。批量作业额外检查 `batch_id` 相对于 `seen_batch_ids` 的唯一性。

**串行执行**由单 worker 循环强制保证：

```rust
loop {
    if let Some(job) = state.queue.lock().await.pop_next() {
        process_job(&state, job).await;  // 完全完成后才处理下一个
        continue;
    }
    state.queue_notify.notified().await;  // 休眠直到被通知
}
```

`print_lock`（`Arc<Mutex<()>>`）提供额外保障：即使存在多个 worker，同一时间也只有一个平台打印命令在执行。目前每个进程只运行一个 worker。

## 作业接受流程

作业从两个来源进入队列：

1. **WebSocket**——`protocol.rs` 校验每个 `print`/`print_batch` 消息（`validate_for_acceptance`），然后调用 `queue.accept_job()` 或 `queue.accept_batch()`。连接立即收到 `JobStatus::Queued`。

2. **远程轮询**——`remote_worker.rs` 校验远程任务并调用 `queue.accept_remote_job()` 或 `queue.accept_remote_batch()`，在 `remote_store.rs` 中进行基于 SQLite 的去重。

两种情况下，`queue_notify.notify_one()` 都会唤醒 worker。

## 作业处理流水线（`process_job_inner`）

出队后，每个作业遵循以下路径：

```
从队列弹出
    │
    ├── 格式 = html / raw-html?
    │   ├── html: 校验 file_url（绝对 http/https）→ 通过 Chrome/Chromium 渲染
    │   └── raw-html: 通过 Chrome/Chromium 渲染内联 HTML
    │       └── 浏览器带过滤代理启动 → CDP PrintToPDF → 临时 PDF → 打印
    │
    ├── 格式 = raw?
    │   ├── 是 → 解码 base64 → 解析打印机 → print_raw() → 跟踪状态
    │
    ├── 下载 file_url 到临时文件（pdf / 图片 / office）
    │   ├── HTTP/HTTPS: 带大小限制的流式下载（Content-Length + 字节计数）
    │   └── data: URL: 直接 base64 解码
    │
    ├── 解析打印机（指定或默认）+ 纸张（指定或默认）
    │
    ├── 如需要则转换为 PDF
    │   ├── Office（docx/xlsx/pptx）→ office_to_pdf()，通过 LibreOffice 或 Windows COM
    │   ├── 图片（PNG/JPEG）→ image_to_pdf()，通过 printpdf crate（适应纸张，203 DPI）
    │   └── PDF → normalize_pdf_path()（确保 .pdf 扩展名以适配打印工具）
    │
    ├── 通过平台后端提交到操作系统打印队列
    │   ├── Windows: SumatraPDF.exe -silent -print-to ...
    │   └── macOS/Linux: lp -d "{printer}" -n {copies} -o media={media}
    │
    ├── 跟踪状态（仅 CUPS）
    │
    └── 清理临时文件
```

任何阶段出现 `Err`，作业将记录为 `JobStatus::Failed` 并附带错误信息。

## 格式检测

PrintBridge 使用 **magic byte 检测**来验证文件内容与声明的格式是否匹配：

| Magic Bytes | 格式 | 检测位置 |
|-------------|------|---------|
| `%PDF-` | PDF | `document.rs` |
| `\x89PNG\r\n\x1a\n` | PNG | `document.rs` |
| `\xFF\xD8\xFF` | JPEG | `document.rs` |
| 含 `word/document.xml` 的 ZIP | Docx | `office.rs` |
| 含 `xl/workbook.xml` 的 ZIP | Xlsx | `office.rs` |
| 含 `ppt/presentation.xml` 的 ZIP | Pptx | `office.rs` |

如果声明的格式与检测到的字节不匹配 → 返回 `FORMAT_MISMATCH` 错误。

## 图片 → PDF 转换

图片通过 `printpdf` crate（`document.rs`，`image_to_pdf`）转换为单页 PDF：

- 图片**适应包含**到目标纸张尺寸（居中，保持宽高比）
- 默认 DPI 假设：**203 DPI**（标签打印机标准）
- 纸张尺寸来自作业的 `paper` 字段或配置默认值

## Office → PDF 转换

Office 文档（docx/xlsx/pptx）通过平台原生 Office 软件转换为 PDF（`office.rs` + `office/`）。在 macOS/Linux 上，在隔离的 profile 中调用 LibreOffice（`soffice`/`libreoffice`），宏安全级别设为最高。在 Windows 上，使用原生 Windows COM 接口。转换有 120 秒超时。打印结果取决于 LibreOffice 的渲染——不保证与 Microsoft Office 或 WPS 完全一致。

## HTML 渲染流水线

HTML 和 raw-html 作业跳过下载阶段，由 `HtmlRenderer`（默认：`BrowserHtmlRenderer`）渲染为临时 PDF：

1. **浏览器发现**——在平台特定位置查找 Chrome、Chromium 或 Edge
2. **过滤代理**——本地 HTTP 代理拦截所有浏览器资源请求。`ResourcePolicy` 阻止非公共 IP（回环、私有、链路本地、多播、`file:`、`data:` 协议）。连接前解析 DNS 以防止 DNS 重绑定。
3. **渲染**——浏览器导航到目标 URL（或加载内联 HTML），等待 `wait_ms` 毫秒（默认 1000，最大 30000），然后通过 CDP `Page.printToPDF` 导出为 PDF。
4. 生成的临时 PDF 通过常规 PDF 打印路径提交，然后清理。

如果检测到被阻止的资源，渲染将以 `HtmlRenderError::BlockedResource` 失败，作业标记为失败。

## 下载安全（`download.rs`）

`download_to_temp` 在两个层面实施安全保护：
1. **Content-Length 头检查**——如果头超过限制，在下载开始前即拒绝
2. **流式字节限制**——在流式下载过程中也强制执行限制

下载有可配置的超时（`limits.download_timeout_seconds`，默认 30 秒）。出错时清理部分下载文件。

支持的 URL 协议：`http://`、`https://`、`data:application/pdf;base64,...`。

## 状态跟踪

作业提交到操作系统打印队列后：

| 平台 | 是否跟踪 | 方式 |
|------|---------|------|
| macOS/Linux | **是** | `lpstat -W completed -o`——检查系统作业 ID 是否出现在已完成列表中 |
| Windows | **否** | SumatraPDF 和 Win32 Spooler API 不暴露可跟踪的状态；`tracking_supported: false` |

在 macOS/Linux 上，跟踪结果可能是 `Completed`、`Failed` 或 `Unknown`（如果找不到作业）。

## 去重与幂等性

- **WebSocket：** 内存中 `QueueState` 的 `seen_job_ids`——进程生命周期内持久
- **远程轮询：** `remote.sqlite3` 中的 `remote_jobs` 表——跨重启持久化（见[远程任务轮询](remote-polling.md)）

两者均使用 `job_id` 作为去重键。

## 源码参考

| 领域 | 文件 |
|------|------|
| 队列状态 + worker 循环 + 作业处理 | `crates/runtime/src/queue.rs` |
| 格式检测 + 图片→PDF | `crates/runtime/src/document.rs` |
| Office 检测 + 转换 | `crates/runtime/src/office.rs`、`office/libreoffice.rs`、`office/windows.rs` |
| HTML 渲染（浏览器、代理、策略） | `crates/runtime/src/html/` |
| 下载到临时文件 | `crates/runtime/src/download.rs` |
| 平台打印后端 | `crates/runtime/src/printing/mod.rs`、`cups.rs`、`windows.rs` |
| 消息校验 + 作业类型 | `crates/core/src/protocol.rs` |
