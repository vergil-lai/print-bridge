use print_bridge_lib::{
    protocol::SupportedFormat,
    remote_protocol::{parse_remote_tasks, RemoteTask},
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
                "format": "image",
                "file_url": "https://example.com/b.jpg",
                "copies": 2
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
        }
        other => panic!("expected print_batch task, got {other:?}"),
    }
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
