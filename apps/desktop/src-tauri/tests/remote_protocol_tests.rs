use print_bridge_lib::{
    protocol::SupportedFormat,
    remote_protocol::{parse_remote_tasks, RemoteProtocolError, RemoteTask},
};

#[test]
fn parses_single_remote_print_task() {
    let json = r#"{
        "type": "print",
        "request_id": "REQ-001",
        "job_id": "JOB-001",
        "format": "pdf",
        "file_url": "https://example.com/label.pdf",
        "copies": 1
    }"#;

    let tasks = parse_remote_tasks(json).unwrap();

    assert_eq!(tasks.len(), 1);
    match &tasks[0] {
        RemoteTask::Print { request_id, job } => {
            assert_eq!(request_id, "REQ-001");
            assert_eq!(job.job_id, "JOB-001");
            assert_eq!(job.format, SupportedFormat::Pdf);
        }
        other => panic!("expected print task, got {other:?}"),
    }
}

#[test]
fn parses_single_remote_html_print_task() {
    let json = r#"{
        "type": "print",
        "request_id": "REQ-HTML-001",
        "job_id": "JOB-HTML-001",
        "format": "html",
        "file_url": "https://example.com/invoice/1",
        "wait_ms": 1500
    }"#;

    let tasks = parse_remote_tasks(json).unwrap();

    match &tasks[0] {
        RemoteTask::Print { job, .. } => {
            assert_eq!(job.format, SupportedFormat::Html);
            assert_eq!(job.wait_ms, Some(1500));
        }
        other => panic!("expected print task, got {other:?}"),
    }
}

#[test]
fn rejects_unsafe_html_url_in_single_remote_task() {
    let json = r#"{
        "type": "print",
        "request_id": "REQ-HTML-FILE",
        "job_id": "JOB-HTML-FILE",
        "format": "html",
        "file_url": "file:///tmp/invoice.html"
    }"#;

    assert!(matches!(
        parse_remote_tasks(json),
        Err(RemoteProtocolError::InvalidMessage(_))
    ));
}

#[test]
fn parses_remote_print_batch_task() {
    let json = r#"{
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
                "format": "raw-html",
                "html": "<main>batch invoice</main>",
                "wait_ms": 0
            }
        ]
    }"#;

    let tasks = parse_remote_tasks(json).unwrap();

    assert_eq!(tasks.len(), 1);
    match &tasks[0] {
        RemoteTask::PrintBatch {
            request_id,
            batch_id,
            jobs,
        } => {
            assert_eq!(request_id, "REQ-002");
            assert_eq!(batch_id, "BATCH-001");
            assert_eq!(jobs.len(), 2);
            assert_eq!(jobs[1].format, SupportedFormat::RawHtml);
            assert_eq!(jobs[1].html.as_deref(), Some("<main>batch invoice</main>"));
        }
        other => panic!("expected print_batch task, got {other:?}"),
    }
}

#[test]
fn rejects_unsafe_html_url_in_remote_batch() {
    let json = r#"{
        "type": "print_batch",
        "request_id": "REQ-HTML-DATA",
        "batch_id": "BATCH-HTML-DATA",
        "jobs": [{
            "job_id": "JOB-HTML-DATA",
            "format": "html",
            "file_url": "data:text/html,<main>invoice</main>"
        }]
    }"#;

    assert!(matches!(
        parse_remote_tasks(json),
        Err(RemoteProtocolError::InvalidMessage(_))
    ));
}

#[test]
fn accepts_empty_remote_task_response() {
    assert!(parse_remote_tasks("").unwrap().is_empty());
    assert!(parse_remote_tasks("null").unwrap().is_empty());
    assert!(parse_remote_tasks("[]").unwrap().is_empty());
}

#[test]
fn rejects_non_print_remote_messages() {
    let json = r#"{ "type": "ping", "time": 123 }"#;

    assert!(parse_remote_tasks(json).is_err());
}
