use crate::protocol::{validate_html_file_url, PrintJobInput, SupportedFormat};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use thiserror::Error;

/// 已进入本地 FIFO 队列的打印任务。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedJob {
    pub request_id: String,
    pub batch_id: Option<String>,
    pub job: PrintJobInput,
    #[serde(default)]
    pub html_local_origin: Option<String>,
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
    #[error("invalid message")]
    InvalidMessage,
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
            html_local_origin: None,
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
            html_local_origin: None,
            remote: true,
        })
    }

    fn accept_queued_job(&mut self, queued_job: QueuedJob) -> Result<(), QueueError> {
        validate_html_source(&queued_job.job)?;
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

    /// 接收来自浏览器连接的任务，并保存已验证的本机 HTML Origin。
    pub fn accept_job_with_html_local_origin(
        &mut self,
        request_id: String,
        job: PrintJobInput,
        html_local_origin: Option<String>,
    ) -> Result<(), QueueError> {
        let job_html_local_origin = html_local_origin_for_job(&job, &html_local_origin);
        self.accept_queued_job(QueuedJob {
            request_id,
            batch_id: None,
            job,
            html_local_origin: job_html_local_origin,
            remote: false,
        })
    }

    /// 接收来自浏览器连接的批量任务，并保存已验证的本机 HTML Origin。
    pub fn accept_batch_with_html_local_origin(
        &mut self,
        request_id: String,
        batch_id: String,
        jobs: Vec<PrintJobInput>,
        html_local_origin: Option<String>,
    ) -> Result<(), QueueError> {
        self.accept_batch_with_html_origin(request_id, batch_id, jobs, false, html_local_origin)
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
        self.accept_batch_with_html_origin(request_id, batch_id, jobs, remote, None)
    }

    fn accept_batch_with_html_origin(
        &mut self,
        request_id: String,
        batch_id: String,
        jobs: Vec<PrintJobInput>,
        remote: bool,
        html_local_origin: Option<String>,
    ) -> Result<(), QueueError> {
        for job in &jobs {
            validate_html_source(job)?;
        }
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
            let job_html_local_origin = html_local_origin_for_job(&job, &html_local_origin);
            self.pending.push_back(QueuedJob {
                request_id: request_id.clone(),
                batch_id: Some(batch_id.clone()),
                job,
                html_local_origin: job_html_local_origin,
                remote,
            });
        }

        Ok(())
    }

    /// 按 FIFO 顺序弹出下一个 worker 要处理的任务。
    pub fn pop_next(&mut self) -> Option<QueuedJob> {
        self.pending.pop_front()
    }

    /// 返回当前尚未被 worker 取走的内存队列快照。
    pub fn pending_jobs(&self) -> Vec<QueuedJob> {
        self.pending.iter().cloned().collect()
    }
}

/// 只把已验证的本机 Origin 附加到与其完全同源的 HTML 作业。
fn html_local_origin_for_job(
    job: &PrintJobInput,
    html_local_origin: &Option<String>,
) -> Option<String> {
    let file_url = job.file_url.as_deref()?;
    let origin = validate_html_file_url(file_url)
        .ok()?
        .origin()
        .ascii_serialization();
    html_local_origin
        .as_ref()
        .filter(|allowed_origin| **allowed_origin == origin)
        .cloned()
}

fn validate_html_source(job: &PrintJobInput) -> Result<(), QueueError> {
    if job.format != SupportedFormat::Html {
        return Ok(());
    }

    let file_url = job
        .file_url
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(QueueError::InvalidMessage)?;
    validate_html_file_url(file_url)
        .map(|_| ())
        .map_err(|_| QueueError::InvalidMessage)
}
