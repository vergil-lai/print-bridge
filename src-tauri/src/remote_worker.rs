use crate::{
    app_state::AppState,
    config::RemoteConfig,
    protocol::PrintJobInput,
    queue::QueueError,
    remote_client::{RemoteClient, RemoteClientError},
    remote_protocol::RemoteTask,
    remote_store::{DeliveryFailureOutcome, NewRemoteJob, RemoteStore},
};
use thiserror::Error;
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PollOutcome {
    pub received: usize,
    pub enqueued: usize,
    pub duplicated: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReportOutcome {
    pub delivered: usize,
    pub will_retry: usize,
    pub abandoned: usize,
}

#[derive(Debug, Error)]
pub enum RemoteWorkerError {
    #[error(transparent)]
    Client(#[from] RemoteClientError),
    #[error(transparent)]
    Store(#[from] rusqlite::Error),
    #[error(transparent)]
    Queue(#[from] QueueError),
    #[error("remote store is not initialized")]
    MissingStore,
}

pub async fn run_worker(state: AppState) {
    let client = RemoteClient::default();

    loop {
        let config = state.config.read().await.remote.clone();
        if !config.enabled {
            state.remote_notify.notified().await;
            continue;
        }

        let poll_result = poll_once(&state, &client).await;
        let report_result = report_pending_once(&state, &client).await;
        if is_configuration_error(&poll_result) || is_configuration_error(&report_result) {
            state.remote_notify.notified().await;
            continue;
        }

        tokio::time::sleep(std::time::Duration::from_secs(
            config.poll_interval_seconds.max(1),
        ))
        .await;
    }
}

pub async fn poll_once(
    state: &AppState,
    client: &RemoteClient,
) -> Result<PollOutcome, RemoteWorkerError> {
    let config = state.config.read().await.remote.clone();
    if !config.enabled {
        return Ok(PollOutcome::default());
    }
    let store = state
        .remote_store
        .as_ref()
        .ok_or(RemoteWorkerError::MissingStore)?;
    let now = now_string();
    let tasks = client.fetch_tasks(&config).await?;
    let mut outcome = PollOutcome {
        received: tasks.len(),
        ..PollOutcome::default()
    };

    for task in tasks {
        match task {
            RemoteTask::Print { request_id, job } => {
                if enqueue_remote_job(state, store, &now, request_id, None, job).await? {
                    outcome.enqueued += 1;
                } else {
                    outcome.duplicated += 1;
                }
            }
            RemoteTask::PrintBatch {
                request_id,
                batch_id,
                jobs,
            } => {
                let mut new_jobs = Vec::new();
                for job in jobs {
                    if record_remote_job(store, &now, &request_id, Some(&batch_id), &job)? {
                        new_jobs.push(job);
                    } else {
                        outcome.duplicated += 1;
                    }
                }
                if !new_jobs.is_empty() {
                    outcome.enqueued += new_jobs.len();
                    state.queue.lock().await.accept_remote_batch(
                        request_id,
                        batch_id,
                        new_jobs,
                    )?;
                    state.queue_notify.notify_one();
                }
            }
        }
    }

    Ok(outcome)
}

pub async fn report_pending_once(
    state: &AppState,
    client: &RemoteClient,
) -> Result<ReportOutcome, RemoteWorkerError> {
    let config = state.config.read().await.remote.clone();
    if !config.enabled {
        return Ok(ReportOutcome::default());
    }
    let store = state
        .remote_store
        .as_ref()
        .ok_or(RemoteWorkerError::MissingStore)?;
    let now = now_string();
    let events = store.pending_status_events(&now, 20)?;
    let mut outcome = ReportOutcome::default();

    for event in events {
        match client.report_status(&config, &event).await {
            Ok(()) => {
                store.mark_delivered(&event.event_id, &now)?;
                outcome.delivered += 1;
            }
            Err(error) if error.is_configuration_status() => {
                return Err(RemoteWorkerError::Client(error));
            }
            Err(error) => {
                let retry_at = retry_at_string(&config);
                match store.mark_delivery_failed(
                    &event.event_id,
                    &retry_at,
                    &error.to_string(),
                    config.max_report_retries.max(1),
                )? {
                    DeliveryFailureOutcome::WillRetry => outcome.will_retry += 1,
                    DeliveryFailureOutcome::Abandoned => outcome.abandoned += 1,
                }
            }
        }
    }

    Ok(outcome)
}

async fn enqueue_remote_job(
    state: &AppState,
    store: &RemoteStore,
    now: &str,
    request_id: String,
    batch_id: Option<String>,
    job: PrintJobInput,
) -> Result<bool, RemoteWorkerError> {
    if !record_remote_job(store, now, &request_id, batch_id.as_deref(), &job)? {
        return Ok(false);
    }

    state
        .queue
        .lock()
        .await
        .accept_remote_job(request_id, job)?;
    state.queue_notify.notify_one();
    Ok(true)
}

fn record_remote_job(
    store: &RemoteStore,
    now: &str,
    request_id: &str,
    batch_id: Option<&str>,
    job: &PrintJobInput,
) -> Result<bool, RemoteWorkerError> {
    Ok(store.record_job_if_new(&NewRemoteJob {
        request_id,
        batch_id,
        job_id: &job.job_id,
        first_seen_at: now,
    })?)
}

fn is_configuration_error<T>(result: &Result<T, RemoteWorkerError>) -> bool {
    matches!(
        result,
        Err(RemoteWorkerError::Client(error)) if error.is_configuration_status()
    )
}

fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn retry_at_string(config: &RemoteConfig) -> String {
    let seconds = config.poll_interval_seconds.max(1).min(i64::MAX as u64) as i64;
    (OffsetDateTime::now_utc() + Duration::seconds(seconds))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
