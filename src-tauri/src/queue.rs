use crate::{
    app_state::AppState,
    config::AgentConfig,
    document::{detect_format, image_to_pdf, DocumentError, DocumentFormat},
    download::{download_to_temp, DownloadError},
    logs::TaskLogEntry,
    printing::{paper_name, PaperInfo, PrintError, PrintOptions, PrintTrackingOutcome},
    protocol::{EffectivePaper, JobStatus, PrintJobInput, SupportedFormat},
    remote_store::{NewRemoteStatusEvent, RemoteReportStatus},
    task_history::{NewTaskHistoryEvent, TaskHistorySource, TaskHistoryStatus},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
};
use thiserror::Error;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

/// 已进入本地 FIFO 队列的打印任务。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedJob {
    pub request_id: String,
    pub batch_id: Option<String>,
    pub job: PrintJobInput,
    #[serde(default)]
    pub remote: bool,
}

/// 可映射回协议错误码的队列接收错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum QueueError {
    #[error("duplicate job id")]
    DuplicateJobId,
    #[error("duplicate batch id")]
    DuplicateBatchId,
}

/// 内存队列状态，以及任务和批次的重复保护。
#[derive(Debug, Default, Clone)]
pub struct QueueState {
    pending: VecDeque<QueuedJob>,
    seen_job_ids: HashSet<String>,
    seen_batch_ids: HashSet<String>,
}

impl QueueState {
    /// 检查任务 ID 是否重复后接收单个任务。
    pub fn accept_job(&mut self, request_id: String, job: PrintJobInput) -> Result<(), QueueError> {
        self.accept_queued_job(QueuedJob {
            request_id,
            batch_id: None,
            job,
            remote: false,
        })
    }

    /// 接收远程服务拉取到的单个任务。
    pub fn accept_remote_job(
        &mut self,
        request_id: String,
        job: PrintJobInput,
    ) -> Result<(), QueueError> {
        self.accept_queued_job(QueuedJob {
            request_id,
            batch_id: None,
            job,
            remote: true,
        })
    }

    /// 把一个已构造的队列任务推入 FIFO 队列。
    fn accept_queued_job(&mut self, queued_job: QueuedJob) -> Result<(), QueueError> {
        if self.seen_job_ids.contains(&queued_job.job.job_id) {
            return Err(QueueError::DuplicateJobId);
        }

        self.seen_job_ids.insert(queued_job.job.job_id.clone());
        self.pending.push_back(queued_job);
        Ok(())
    }

    /// 仅当批次和每个任务 ID 都唯一时接收整批任务。
    pub fn accept_batch(
        &mut self,
        request_id: String,
        batch_id: String,
        jobs: Vec<PrintJobInput>,
    ) -> Result<(), QueueError> {
        self.accept_batch_with_remote(request_id, batch_id, jobs, false)
    }

    /// 接收远程服务拉取到的整批任务。
    pub fn accept_remote_batch(
        &mut self,
        request_id: String,
        batch_id: String,
        jobs: Vec<PrintJobInput>,
    ) -> Result<(), QueueError> {
        self.accept_batch_with_remote(request_id, batch_id, jobs, true)
    }

    fn accept_batch_with_remote(
        &mut self,
        request_id: String,
        batch_id: String,
        jobs: Vec<PrintJobInput>,
        remote: bool,
    ) -> Result<(), QueueError> {
        if self.seen_batch_ids.contains(&batch_id) {
            return Err(QueueError::DuplicateBatchId);
        }

        let mut batch_job_ids = HashSet::new();
        for job in &jobs {
            if self.seen_job_ids.contains(&job.job_id) || !batch_job_ids.insert(job.job_id.clone())
            {
                return Err(QueueError::DuplicateJobId);
            }
        }

        self.seen_batch_ids.insert(batch_id.clone());
        for job in jobs {
            self.seen_job_ids.insert(job.job_id.clone());
            self.pending.push_back(QueuedJob {
                request_id: request_id.clone(),
                batch_id: Some(batch_id.clone()),
                job,
                remote,
            });
        }

        Ok(())
    }

    /// 按 FIFO 顺序弹出下一个 worker 要处理的任务。
    pub fn pop_next(&mut self) -> Option<QueuedJob> {
        self.pending.pop_front()
    }
}

/// 转换为状态日志前的 worker 内部错误。
#[derive(Debug, Error)]
enum ProcessJobError {
    #[error("printer not configured")]
    PrinterNotConfigured,
    #[error("paper not configured")]
    PaperNotConfigured,
    #[error("copies out of range")]
    CopiesOutOfRange,
    #[error("unsupported document format")]
    UnsupportedFormat,
    #[error("format mismatch: expected {expected}, got {actual}")]
    FormatMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("download failed: {0}")]
    Download(#[from] DownloadError),
    #[error("document normalization failed: {0}")]
    Document(#[from] DocumentError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("print failed: {0}")]
    Print(#[from] PrintError),
}

/// 运行后台 worker 循环，等待并处理队列任务。
pub async fn run_worker(state: AppState) {
    loop {
        let next_job = state.queue.lock().await.pop_next();

        if let Some(queued_job) = next_job {
            process_job(&state, queued_job).await;
            continue;
        }

        state.queue_notify.notified().await;
    }
}

/// 处理一个队列任务，并在任务日志中记录成功或失败。
pub async fn process_job(state: &AppState, queued_job: QueuedJob) {
    push_log(state, &queued_job, JobStatus::Queued, "queued").await;

    if let Err(error) = process_job_inner(state, &queued_job).await {
        push_log(state, &queued_job, JobStatus::Failed, &error.to_string()).await;
    }
}

/// 解析配置、下载文件、执行打印，并清理下载文件。
async fn process_job_inner(
    state: &AppState,
    queued_job: &QueuedJob,
) -> Result<(), ProcessJobError> {
    let config = state.config.read().await.clone();
    let options = resolve_print_options(queued_job, &config)?;

    push_log(state, queued_job, JobStatus::Downloading, "downloading").await;
    let downloaded_path = download_to_temp(&queued_job.job.file_url, &config.limits).await?;
    let result = print_downloaded_file(state, queued_job, &options, &downloaded_path).await;
    cleanup_file(&downloaded_path).await;
    result
}

/// 必要时转换下载文件，并提交给打印后端。
async fn print_downloaded_file(
    state: &AppState,
    queued_job: &QueuedJob,
    options: &PrintOptions,
    downloaded_path: &Path,
) -> Result<(), ProcessJobError> {
    let printable_path =
        prepare_printable_pdf(downloaded_path, queued_job.job.format, &options.paper).await?;

    push_log_with_metadata(
        state,
        queued_job,
        JobStatus::Printing,
        "printing",
        Some(options),
    )
    .await;
    let print_result = {
        let _print_guard = state.print_lock.lock().await;
        state.printing.print_pdf(&printable_path, options)
    };

    // 转换后的图片会生成第二个临时 PDF，原始 PDF 则复用下载路径。
    if printable_path != downloaded_path {
        cleanup_file(&printable_path).await;
    }

    let submission = print_result?;
    push_log_with_metadata(
        state,
        queued_job,
        JobStatus::Submitted,
        "submitted to system print queue",
        Some(options),
    )
    .await;

    let (tracking_status, tracking_message) =
        match state.printing.track_submission(&submission, options) {
            PrintTrackingOutcome::Completed { message } => (JobStatus::Completed, message),
            PrintTrackingOutcome::Failed { message } => (JobStatus::Failed, message),
            PrintTrackingOutcome::Unknown { message } => (JobStatus::Unknown, message),
        };
    push_log_with_metadata_without_remote(
        state,
        queued_job,
        tracking_status,
        &tracking_message,
        Some(options),
    )
    .await;

    Ok(())
}

/// 解析队列任务的打印机、纸张和份数设置。
fn resolve_print_options(
    queued_job: &QueuedJob,
    config: &AgentConfig,
) -> Result<PrintOptions, ProcessJobError> {
    let printer_name = config
        .printing
        .default_printer
        .clone()
        .ok_or(ProcessJobError::PrinterNotConfigured)?;
    let paper = queued_job
        .job
        .paper
        .clone()
        .or_else(|| config.printing.default_paper.clone())
        .ok_or(ProcessJobError::PaperNotConfigured)?;

    if queued_job.job.copies == 0 || queued_job.job.copies > config.limits.max_copies {
        return Err(ProcessJobError::CopiesOutOfRange);
    }

    Ok(PrintOptions {
        printer_name,
        paper: paper_info_from_effective(&paper),
        copies: queued_job.job.copies,
    })
}

/// 确保下载文档是可打印 PDF，并与请求格式一致。
async fn prepare_printable_pdf(
    downloaded_path: &Path,
    expected_format: SupportedFormat,
    paper: &PaperInfo,
) -> Result<PathBuf, ProcessJobError> {
    let actual_format =
        detect_format(downloaded_path)?.ok_or(ProcessJobError::UnsupportedFormat)?;
    if !format_matches(expected_format, actual_format) {
        return Err(ProcessJobError::FormatMismatch {
            expected: supported_format_name(expected_format),
            actual: document_format_name(actual_format),
        });
    }

    match actual_format {
        DocumentFormat::Pdf => normalize_pdf_path(downloaded_path).await,
        DocumentFormat::Png | DocumentFormat::Jpeg => {
            let output_path = downloaded_path.with_extension("pdf");
            image_to_pdf(
                downloaded_path,
                &EffectivePaper {
                    width_mm: paper.width_mm,
                    height_mm: paper.height_mm,
                },
                &output_path,
            )?;
            Ok(output_path)
        }
    }
}

/// 为无扩展名的已下载 PDF 提供 .pdf 路径，适配需要扩展名的打印工具。
async fn normalize_pdf_path(downloaded_path: &Path) -> Result<PathBuf, ProcessJobError> {
    if downloaded_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        return Ok(downloaded_path.to_path_buf());
    }

    let output_path = downloaded_path.with_extension("pdf");
    tokio::fs::copy(downloaded_path, &output_path).await?;
    Ok(output_path)
}

/// 检查任务声明格式是否与检测到的文档字节一致。
fn format_matches(expected: SupportedFormat, actual: DocumentFormat) -> bool {
    matches!(
        (expected, actual),
        (SupportedFormat::Pdf, DocumentFormat::Pdf)
            | (
                SupportedFormat::Image,
                DocumentFormat::Png | DocumentFormat::Jpeg
            )
            | (SupportedFormat::Png, DocumentFormat::Png)
            | (
                SupportedFormat::Jpg | SupportedFormat::Jpeg,
                DocumentFormat::Jpeg
            )
    )
}

/// 返回请求格式在协议中的拼写。
fn supported_format_name(format: SupportedFormat) -> &'static str {
    match format {
        SupportedFormat::Pdf => "pdf",
        SupportedFormat::Image => "image",
        SupportedFormat::Png => "png",
        SupportedFormat::Jpg => "jpg",
        SupportedFormat::Jpeg => "jpeg",
    }
}

/// 返回格式不匹配错误中使用的检测格式名称。
fn document_format_name(format: DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::Pdf => "pdf",
        DocumentFormat::Png => "png",
        DocumentFormat::Jpeg => "jpeg",
    }
}

/// 把协议中的纸张尺寸转换为打印后端使用的纸张结构。
fn paper_info_from_effective(paper: &EffectivePaper) -> PaperInfo {
    PaperInfo {
        id: format!(
            "custom_{}x{}mm",
            format_paper_dimension(paper.width_mm),
            format_paper_dimension(paper.height_mm)
        ),
        name: paper_name(paper.width_mm, paper.height_mm),
        width_mm: paper.width_mm,
        height_mm: paper.height_mm,
    }
}

/// 格式化纸张尺寸，去掉不必要的小数尾零。
fn format_paper_dimension(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

/// 删除临时文件，并忽略清理失败。
async fn cleanup_file(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

/// 保存任务日志记录，并广播给已订阅的 WebSocket 客户端。
async fn push_log(state: &AppState, queued_job: &QueuedJob, status: JobStatus, message: &str) {
    push_log_with_metadata(state, queued_job, status, message, None).await;
}

async fn push_log_with_metadata(
    state: &AppState,
    queued_job: &QueuedJob,
    status: JobStatus,
    message: &str,
    options: Option<&PrintOptions>,
) {
    push_log_with_metadata_and_remote(state, queued_job, status, message, options, true).await;
}

async fn push_log_with_metadata_without_remote(
    state: &AppState,
    queued_job: &QueuedJob,
    status: JobStatus,
    message: &str,
    options: Option<&PrintOptions>,
) {
    push_log_with_metadata_and_remote(state, queued_job, status, message, options, false).await;
}

async fn push_log_with_metadata_and_remote(
    state: &AppState,
    queued_job: &QueuedJob,
    status: JobStatus,
    message: &str,
    options: Option<&PrintOptions>,
    enqueue_remote: bool,
) {
    let entry = TaskLogEntry {
        timestamp: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
        request_id: Some(queued_job.request_id.clone()),
        batch_id: queued_job.batch_id.clone(),
        job_id: Some(queued_job.job.job_id.clone()),
        origin: None,
        status,
        message: message.to_string(),
    };
    let occurred_at = entry.timestamp.clone();
    state.logs.lock().await.push(entry.clone());
    state.broadcast_status(entry);

    if let Some(task_history) = &state.task_history {
        let result = task_history.record_event(&NewTaskHistoryEvent {
            job_id: &queued_job.job.job_id,
            request_id: Some(&queued_job.request_id),
            batch_id: queued_job.batch_id.as_deref(),
            source: task_history_source(queued_job),
            status: task_history_status(status),
            message: if message.is_empty() {
                None
            } else {
                Some(message)
            },
            printer_name: options.map(|value| value.printer_name.as_str()),
            paper_name: options.map(|value| value.paper.name.as_str()),
            copies: Some(queued_job.job.copies),
            occurred_at: &occurred_at,
        });
        if let Err(error) = result {
            tauri_plugin_log::log::error!("failed to record task history event: {error}");
        }
    }

    if enqueue_remote {
        enqueue_remote_status_event(state, queued_job, status, message).await;
    }
}

fn task_history_status(status: JobStatus) -> TaskHistoryStatus {
    match status {
        JobStatus::Queued => TaskHistoryStatus::Queued,
        JobStatus::Downloading => TaskHistoryStatus::Downloading,
        JobStatus::Printing => TaskHistoryStatus::Printing,
        JobStatus::Submitted => TaskHistoryStatus::Submitted,
        JobStatus::Completed => TaskHistoryStatus::Completed,
        JobStatus::Failed => TaskHistoryStatus::Failed,
        JobStatus::Unknown => TaskHistoryStatus::Unknown,
        JobStatus::Cancelled => TaskHistoryStatus::Cancelled,
    }
}

fn task_history_source(queued_job: &QueuedJob) -> TaskHistorySource {
    if queued_job.remote {
        TaskHistorySource::Remote
    } else {
        TaskHistorySource::WebSocket
    }
}

async fn enqueue_remote_status_event(
    state: &AppState,
    queued_job: &QueuedJob,
    status: JobStatus,
    message: &str,
) {
    if !queued_job.remote {
        return;
    }
    let Some(remote_status) = remote_status_for_job_status(status) else {
        return;
    };
    let Some(store) = &state.remote_store else {
        return;
    };
    let occurred_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
    let _ = store.insert_status_event(&NewRemoteStatusEvent {
        job_id: &queued_job.job.job_id,
        request_id: &queued_job.request_id,
        batch_id: queued_job.batch_id.as_deref(),
        status: remote_status,
        message: if message.is_empty() {
            None
        } else {
            Some(message)
        },
        occurred_at: &occurred_at,
        next_retry_at: &occurred_at,
    });
}

fn remote_status_for_job_status(status: JobStatus) -> Option<RemoteReportStatus> {
    match status {
        JobStatus::Queued => Some(RemoteReportStatus::Accepted),
        JobStatus::Submitted => Some(RemoteReportStatus::Success),
        JobStatus::Failed | JobStatus::Cancelled => Some(RemoteReportStatus::Failed),
        JobStatus::Downloading
        | JobStatus::Printing
        | JobStatus::Completed
        | JobStatus::Unknown => None,
    }
}

#[cfg(test)]
mod worker_tests {
    use super::{process_job, resolve_print_options, QueuedJob};
    use crate::{
        app_state::AppState,
        config::{AgentConfig, PrintingConfig},
        printing::{
            PaperInfo, PrintBackend, PrintOptions, PrintResult, PrintSubmission,
            PrintTrackingOutcome, PrinterInfo,
        },
        protocol::{EffectivePaper, JobStatus, PrintJobInput, SupportedFormat},
        remote_store::{RemoteReportStatus, RemoteStore},
        task_history::{TaskHistoryStatus, TaskHistoryStore},
    };
    use image::{ImageBuffer, Rgb};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    #[test]
    fn resolve_print_options_prefers_job_paper_over_default_paper() {
        let config = config_with_defaults(Some(EffectivePaper {
            width_mm: 80.0,
            height_mm: 50.0,
        }));
        let queued = queued_job(job_with(
            "job-paper",
            SupportedFormat::Pdf,
            "http://127.0.0.1/file.pdf",
            2,
            Some(EffectivePaper {
                width_mm: 40.0,
                height_mm: 30.0,
            }),
        ));

        let options = resolve_print_options(&queued, &config).unwrap();

        assert_eq!(options.printer_name, "Printer A");
        assert_eq!(options.copies, 2);
        assert_eq!(options.paper.width_mm, 40.0);
        assert_eq!(options.paper.height_mm, 30.0);
    }

    #[test]
    fn resolve_print_options_uses_default_paper_when_job_has_none() {
        let config = config_with_defaults(Some(EffectivePaper {
            width_mm: 80.0,
            height_mm: 50.0,
        }));
        let queued = queued_job(job_with(
            "default-paper",
            SupportedFormat::Pdf,
            "http://127.0.0.1/file.pdf",
            1,
            None,
        ));

        let options = resolve_print_options(&queued, &config).unwrap();

        assert_eq!(options.paper.width_mm, 80.0);
        assert_eq!(options.paper.height_mm, 50.0);
    }

    #[tokio::test]
    async fn process_job_logs_failed_without_panicking_when_printer_or_paper_is_missing() {
        let mut missing_printer_config = AgentConfig::default();
        missing_printer_config.printing.default_paper = Some(EffectivePaper {
            width_mm: 80.0,
            height_mm: 50.0,
        });
        let missing_printer_state = AppState::with_printing(
            missing_printer_config,
            Box::new(MockPrintBackend::default()),
        );

        process_job(
            &missing_printer_state,
            queued_job(job_with(
                "missing-printer",
                SupportedFormat::Pdf,
                "http://127.0.0.1/file.pdf",
                1,
                None,
            )),
        )
        .await;

        let printer_logs = missing_printer_state.logs.lock().await.recent();
        assert!(printer_logs
            .iter()
            .any(|entry| entry.status == JobStatus::Failed
                && entry.message.contains("printer not configured")));

        let missing_paper_config = config_with_defaults(None);
        let missing_paper_state =
            AppState::with_printing(missing_paper_config, Box::new(MockPrintBackend::default()));

        process_job(
            &missing_paper_state,
            queued_job(job_with(
                "missing-paper",
                SupportedFormat::Pdf,
                "http://127.0.0.1/file.pdf",
                1,
                None,
            )),
        )
        .await;

        let paper_logs = missing_paper_state.logs.lock().await.recent();
        assert!(paper_logs
            .iter()
            .any(|entry| entry.status == JobStatus::Failed
                && entry.message.contains("paper not configured")));
    }

    #[tokio::test]
    async fn remote_process_job_creates_remote_status_events() {
        let mut missing_printer_config = AgentConfig::default();
        missing_printer_config.printing.default_paper = Some(EffectivePaper {
            width_mm: 80.0,
            height_mm: 50.0,
        });
        let store = RemoteStore::open_in_memory().unwrap();
        let state = AppState::with_printing(
            missing_printer_config,
            Box::new(MockPrintBackend::default()),
        )
        .with_remote_store(store);
        let queued = QueuedJob {
            request_id: "REQ-001".to_string(),
            batch_id: None,
            job: job_with(
                "JOB-001",
                SupportedFormat::Pdf,
                "http://127.0.0.1/file.pdf",
                1,
                None,
            ),
            remote: true,
        };

        process_job(&state, queued).await;

        let events = state
            .remote_store
            .as_ref()
            .unwrap()
            .pending_status_events("9999-01-01T00:00:00Z", 10)
            .unwrap();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|event| event.status == RemoteReportStatus::Accepted));
        assert!(events
            .iter()
            .any(|event| event.status == RemoteReportStatus::Failed));
    }

    #[test]
    fn remote_status_for_job_status_maps_submitted_to_success() {
        assert_eq!(
            super::remote_status_for_job_status(JobStatus::Submitted),
            Some(RemoteReportStatus::Success)
        );
    }

    #[test]
    fn remote_status_for_job_status_ignores_completed_and_unknown() {
        assert_eq!(
            super::remote_status_for_job_status(JobStatus::Completed),
            None
        );
        assert_eq!(
            super::remote_status_for_job_status(JobStatus::Unknown),
            None
        );
    }

    #[tokio::test]
    async fn process_downloaded_job_prints_pdf_with_mock_backend() {
        let pdf_path = temp_path("worker-pdf-source.tmp");
        let _ = fs::remove_file(&pdf_path);
        let pdf_output_path = pdf_path.with_extension("pdf");
        let _ = fs::remove_file(&pdf_output_path);
        fs::write(&pdf_path, b"%PDF-1.7\n%%EOF").unwrap();

        let backend = MockPrintBackend::default();
        let calls = backend.calls.clone();
        let state = AppState::with_printing(
            config_with_defaults(Some(default_paper())),
            Box::new(backend),
        );
        let queued = queued_job(job_with(
            "pdf-job",
            SupportedFormat::Pdf,
            "http://127.0.0.1/file.pdf",
            3,
            None,
        ));
        let config = state.config.read().await.clone();
        let options = resolve_print_options(&queued, &config).unwrap();

        super::print_downloaded_file(&state, &queued, &options, &pdf_path)
            .await
            .unwrap();

        {
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].options.copies, 3);
            assert_eq!(calls[0].options.paper.width_mm, 80.0);
            assert_eq!(calls[0].path.extension().unwrap(), "pdf");
            assert!(calls[0].path_bytes.starts_with(b"%PDF-"));
        }

        let logs = state.logs.lock().await.recent();
        assert!(logs
            .iter()
            .any(|entry| entry.status == JobStatus::Submitted));
        let _ = fs::remove_file(&pdf_path);
        let _ = fs::remove_file(&pdf_output_path);
    }

    #[tokio::test]
    async fn process_downloaded_job_records_submitted_and_unknown_tracking_history() {
        let pdf_path = temp_path("worker-history-source.tmp");
        let _ = fs::remove_file(&pdf_path);
        let pdf_output_path = pdf_path.with_extension("pdf");
        let _ = fs::remove_file(&pdf_output_path);
        fs::write(&pdf_path, b"%PDF-1.7\n%%EOF").unwrap();

        let state = AppState::with_printing(
            config_with_defaults(Some(default_paper())),
            Box::new(MockPrintBackend::default()),
        )
        .with_task_history_store(TaskHistoryStore::open_in_memory().unwrap());
        let queued = queued_job(job_with(
            "print-ok",
            SupportedFormat::Pdf,
            "http://127.0.0.1/file.pdf",
            2,
            None,
        ));
        let config = state.config.read().await.clone();
        let options = resolve_print_options(&queued, &config).unwrap();

        super::print_downloaded_file(&state, &queued, &options, &pdf_path)
            .await
            .unwrap();

        let jobs = state
            .task_history
            .as_ref()
            .unwrap()
            .recent_jobs(500)
            .unwrap();
        assert_eq!(jobs[0].current_status, TaskHistoryStatus::Unknown);
        assert_eq!(jobs[0].printer_name.as_deref(), Some("Printer A"));
        assert_eq!(jobs[0].paper_name.as_deref(), Some("80 x 50 mm"));
        assert_eq!(jobs[0].copies, Some(2));
        let events = state
            .task_history
            .as_ref()
            .unwrap()
            .events_for_job("print-ok")
            .unwrap();
        assert!(events
            .iter()
            .any(|event| event.status == TaskHistoryStatus::Submitted));
        assert!(events
            .iter()
            .any(|event| event.status == TaskHistoryStatus::Unknown));

        let _ = fs::remove_file(&pdf_path);
        let _ = fs::remove_file(&pdf_output_path);
    }

    #[tokio::test]
    async fn remote_downloaded_job_does_not_report_tracking_failure_after_submitted() {
        let pdf_path = temp_path("worker-remote-success-source.tmp");
        let _ = fs::remove_file(&pdf_path);
        let pdf_output_path = pdf_path.with_extension("pdf");
        let _ = fs::remove_file(&pdf_output_path);
        fs::write(&pdf_path, b"%PDF-1.7\n%%EOF").unwrap();

        let backend = MockPrintBackend {
            tracking_outcome: Some(PrintTrackingOutcome::Failed {
                message: "system queue failed after submission".to_string(),
            }),
            ..MockPrintBackend::default()
        };
        let state = AppState::with_printing(
            config_with_defaults(Some(default_paper())),
            Box::new(backend),
        )
        .with_remote_store(RemoteStore::open_in_memory().unwrap());
        let mut queued = queued_job(job_with(
            "remote-print-ok",
            SupportedFormat::Pdf,
            "http://127.0.0.1/file.pdf",
            1,
            None,
        ));
        queued.remote = true;
        let config = state.config.read().await.clone();
        let options = resolve_print_options(&queued, &config).unwrap();

        super::push_log(&state, &queued, JobStatus::Queued, "queued").await;
        super::print_downloaded_file(&state, &queued, &options, &pdf_path)
            .await
            .unwrap();

        let events = state
            .remote_store
            .as_ref()
            .unwrap()
            .pending_status_events("9999-01-01T00:00:00Z", 10)
            .unwrap();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|event| event.status == RemoteReportStatus::Accepted));
        assert!(events
            .iter()
            .any(|event| event.status == RemoteReportStatus::Success));
        assert!(!events
            .iter()
            .any(|event| event.status == RemoteReportStatus::Failed));

        let _ = fs::remove_file(&pdf_path);
        let _ = fs::remove_file(&pdf_output_path);
    }

    #[tokio::test]
    async fn process_downloaded_job_converts_image_to_pdf_before_printing() {
        let image_path = temp_path("worker-image-source.png");
        let _ = fs::remove_file(&image_path);
        let image = ImageBuffer::from_pixel(2, 1, Rgb([255_u8, 0, 0]));
        image.save(&image_path).unwrap();

        let backend = MockPrintBackend::default();
        let calls = backend.calls.clone();
        let state = AppState::with_printing(
            config_with_defaults(Some(default_paper())),
            Box::new(backend),
        );
        let queued = queued_job(job_with(
            "image-job",
            SupportedFormat::Png,
            "http://127.0.0.1/file.png",
            1,
            None,
        ));
        let config = state.config.read().await.clone();
        let options = resolve_print_options(&queued, &config).unwrap();

        super::print_downloaded_file(&state, &queued, &options, &image_path)
            .await
            .unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].path_bytes.starts_with(b"%PDF-"));
        let _ = fs::remove_file(&image_path);
    }

    #[tokio::test]
    async fn process_downloaded_job_rejects_format_mismatch() {
        let image_path = temp_path("worker-format-mismatch.png");
        let _ = fs::remove_file(&image_path);
        let image = ImageBuffer::from_pixel(2, 1, Rgb([255_u8, 0, 0]));
        image.save(&image_path).unwrap();

        let backend = MockPrintBackend::default();
        let calls = backend.calls.clone();
        let state = AppState::with_printing(
            config_with_defaults(Some(default_paper())),
            Box::new(backend),
        );
        let queued = queued_job(job_with(
            "format-mismatch",
            SupportedFormat::Pdf,
            "http://127.0.0.1/file.pdf",
            1,
            None,
        ));
        let config = state.config.read().await.clone();
        let options = resolve_print_options(&queued, &config).unwrap();

        let error = super::print_downloaded_file(&state, &queued, &options, &image_path)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("format mismatch"));
        assert!(calls.lock().unwrap().is_empty());
        let _ = fs::remove_file(&image_path);
    }

    #[derive(Default)]
    struct MockPrintBackend {
        calls: Arc<Mutex<Vec<PrintCall>>>,
        tracking_outcome: Option<PrintTrackingOutcome>,
    }

    struct PrintCall {
        path: PathBuf,
        path_bytes: Vec<u8>,
        options: PrintOptions,
    }

    impl PrintBackend for MockPrintBackend {
        fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
            Ok(vec![])
        }

        fn list_papers(&self, _printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
            Ok(vec![])
        }

        fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<PrintSubmission> {
            self.calls.lock().unwrap().push(PrintCall {
                path: path.to_path_buf(),
                path_bytes: fs::read(path).unwrap(),
                options: options.clone(),
            });
            Ok(mock_submission())
        }

        fn track_submission(
            &self,
            _submission: &PrintSubmission,
            _options: &PrintOptions,
        ) -> PrintTrackingOutcome {
            self.tracking_outcome
                .clone()
                .unwrap_or_else(|| PrintTrackingOutcome::Unknown {
                    message: "platform does not provide trackable print status".to_string(),
                })
        }
    }

    fn mock_submission() -> PrintSubmission {
        PrintSubmission {
            submitted_at: "2026-07-06T00:00:00Z".to_string(),
            backend: "mock".to_string(),
            system_job_id: None,
            tracking_supported: false,
        }
    }

    fn queued_job(job: PrintJobInput) -> QueuedJob {
        QueuedJob {
            request_id: "request-1".to_string(),
            batch_id: None,
            job,
            remote: false,
        }
    }

    fn job_with(
        job_id: &str,
        format: SupportedFormat,
        file_url: &str,
        copies: u16,
        paper: Option<EffectivePaper>,
    ) -> PrintJobInput {
        PrintJobInput {
            job_id: job_id.to_string(),
            format,
            file_url: file_url.to_string(),
            copies,
            paper,
        }
    }

    fn config_with_defaults(default_paper: Option<EffectivePaper>) -> AgentConfig {
        AgentConfig {
            printing: PrintingConfig {
                default_printer: Some("Printer A".to_string()),
                default_paper,
                default_copies: 1,
            },
            ..AgentConfig::default()
        }
    }

    fn default_paper() -> EffectivePaper {
        EffectivePaper {
            width_mm: 80.0,
            height_mm: 50.0,
        }
    }

    fn temp_path(file_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "print-bridge-queue-worker-test-{}-{file_name}",
            std::process::id()
        ))
    }
}
