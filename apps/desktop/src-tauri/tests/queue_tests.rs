use print_bridge_lib::{
    logs::{LogStore, TaskLogEntry},
    protocol::{JobStatus, PrintJobInput, SupportedFormat},
    queue::{QueueError, QueueState},
};

fn job(job_id: &str) -> PrintJobInput {
    PrintJobInput {
        job_id: job_id.to_string(),
        format: SupportedFormat::Pdf,
        printer_name: None,
        file_url: Some(format!("https://example.com/{job_id}.pdf")),
        data_base64: None,
        html: None,
        wait_ms: None,
        copies: Some(1),
        paper: None,
    }
}

#[test]
fn duplicate_job_ids_are_rejected() {
    let mut queue = QueueState::default();

    queue
        .accept_job("request-1".to_string(), job("job-1"))
        .unwrap();
    let result = queue.accept_job("request-2".to_string(), job("job-1"));

    assert_eq!(result, Err(QueueError::DuplicateJobId));
}

#[test]
fn duplicate_batch_ids_are_rejected() {
    let mut queue = QueueState::default();

    queue
        .accept_batch(
            "request-1".to_string(),
            "batch-1".to_string(),
            vec![job("job-1")],
        )
        .unwrap();
    let result = queue.accept_batch(
        "request-2".to_string(),
        "batch-1".to_string(),
        vec![job("job-2")],
    );

    assert_eq!(result, Err(QueueError::DuplicateBatchId));
}

#[test]
fn duplicate_job_inside_batch_is_rejected() {
    let mut queue = QueueState::default();

    let result = queue.accept_batch(
        "request-1".to_string(),
        "batch-1".to_string(),
        vec![job("job-1"), job("job-1")],
    );

    assert_eq!(result, Err(QueueError::DuplicateJobId));
    assert!(queue.pop_next().is_none());
}

#[test]
fn batch_rejects_job_id_seen_from_single_job() {
    let mut queue = QueueState::default();

    queue
        .accept_job("request-1".to_string(), job("job-1"))
        .unwrap();
    let result = queue.accept_batch(
        "request-2".to_string(),
        "batch-1".to_string(),
        vec![job("job-1"), job("job-2")],
    );

    assert_eq!(result, Err(QueueError::DuplicateJobId));
    assert_eq!(queue.pop_next().unwrap().job.job_id, "job-1");
    assert!(queue.pop_next().is_none());
}

#[test]
fn single_job_enqueue_preserves_request_id_without_batch_id() {
    let mut queue = QueueState::default();

    queue
        .accept_job("request-1".to_string(), job("job-1"))
        .unwrap();

    let queued = queue.pop_next().unwrap();
    assert_eq!(queued.request_id, "request-1");
    assert_eq!(queued.batch_id, None);
    assert_eq!(queued.job.job_id, "job-1");
    assert!(queue.pop_next().is_none());
}

#[test]
fn queue_rejects_html_jobs_with_non_http_sources() {
    let mut queue = QueueState::default();
    let mut html = job("html-file");
    html.format = SupportedFormat::Html;
    html.file_url = Some("file:///tmp/invoice.html".to_string());

    assert_eq!(
        queue.accept_job("request-1".to_string(), html),
        Err(QueueError::InvalidMessage)
    );
    assert!(queue.pop_next().is_none());
}

#[test]
fn queue_rejects_html_data_url_in_batch_without_accepting_any_job() {
    let mut queue = QueueState::default();
    let mut html = job("html-data");
    html.format = SupportedFormat::Html;
    html.file_url = Some("data:text/html,<main>invoice</main>".to_string());

    assert_eq!(
        queue.accept_batch(
            "request-1".to_string(),
            "batch-1".to_string(),
            vec![job("pdf-1"), html],
        ),
        Err(QueueError::InvalidMessage)
    );
    assert!(queue.pop_next().is_none());
}

#[test]
fn batch_jobs_pop_in_fifo_order_with_request_and_batch_ids() {
    let mut queue = QueueState::default();

    queue
        .accept_batch(
            "request-1".to_string(),
            "batch-1".to_string(),
            vec![job("job-1"), job("job-2"), job("job-3")],
        )
        .unwrap();

    let first = queue.pop_next().unwrap();
    assert_eq!(first.request_id, "request-1");
    assert_eq!(first.batch_id.as_deref(), Some("batch-1"));
    assert_eq!(first.job.job_id, "job-1");

    let second = queue.pop_next().unwrap();
    assert_eq!(second.request_id, "request-1");
    assert_eq!(second.batch_id.as_deref(), Some("batch-1"));
    assert_eq!(second.job.job_id, "job-2");

    let third = queue.pop_next().unwrap();
    assert_eq!(third.request_id, "request-1");
    assert_eq!(third.batch_id.as_deref(), Some("batch-1"));
    assert_eq!(third.job.job_id, "job-3");

    assert!(queue.pop_next().is_none());
}

#[test]
fn logs_retain_recent_entries_up_to_capacity() {
    let mut logs = LogStore::with_capacity(3);

    for index in 1..=4 {
        logs.push(TaskLogEntry {
            timestamp: format!("2026-07-03T00:00:0{index}Z"),
            request_id: Some(format!("request-{index}")),
            batch_id: None,
            job_id: Some(format!("job-{index}")),
            origin: Some("https://app.example.com".to_string()),
            status: JobStatus::Queued,
            message: format!("queued-{index}"),
        });
    }

    let recent = logs.recent();

    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].message, "queued-2");
    assert_eq!(recent[1].message, "queued-3");
    assert_eq!(recent[2].message, "queued-4");
}
