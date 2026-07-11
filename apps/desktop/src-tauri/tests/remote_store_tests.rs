use print_bridge_lib::remote_store::{
    DeliveryFailureOutcome, NewRemoteJob, NewRemoteStatusEvent, RemoteDeliveryState,
    RemoteReportStatus, RemoteStore,
};

fn store() -> RemoteStore {
    RemoteStore::open_in_memory().unwrap()
}

#[test]
fn records_remote_jobs_once_by_job_id() {
    let store = store();
    let job = NewRemoteJob {
        request_id: "REQ-001",
        batch_id: None,
        job_id: "JOB-001",
        first_seen_at: "2026-07-05T00:00:00Z",
    };

    assert!(store.record_job_if_new(&job).unwrap());
    assert!(!store.record_job_if_new(&job).unwrap());
}

#[test]
fn status_event_gets_uuid_and_is_unique_by_job_and_status() {
    let store = store();
    let event = NewRemoteStatusEvent {
        job_id: "JOB-001",
        request_id: "REQ-001",
        batch_id: None,
        status: RemoteReportStatus::Accepted,
        message: None,
        occurred_at: "2026-07-05T00:00:00Z",
        next_retry_at: "2026-07-05T00:00:00Z",
    };

    let inserted = store.insert_status_event(&event).unwrap().unwrap();
    let duplicate = store.insert_status_event(&event).unwrap();

    assert!(uuid::Uuid::parse_str(&inserted.event_id).is_ok());
    assert_eq!(inserted.delivery_state, RemoteDeliveryState::Pending);
    assert!(duplicate.is_none());
}

#[test]
fn pending_events_are_marked_delivered_or_abandoned_after_retries() {
    let store = store();
    let event = store
        .insert_status_event(&NewRemoteStatusEvent {
            job_id: "JOB-001",
            request_id: "REQ-001",
            batch_id: None,
            status: RemoteReportStatus::Failed,
            message: Some("print failed"),
            occurred_at: "2026-07-05T00:00:00Z",
            next_retry_at: "2026-07-05T00:00:00Z",
        })
        .unwrap()
        .unwrap();

    let pending = store
        .pending_status_events("2026-07-05T00:00:01Z", 10)
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event_id, event.event_id);

    let outcome = store
        .mark_delivery_failed(
            &event.event_id,
            "2026-07-05T00:00:10Z",
            "server returned 500",
            2,
        )
        .unwrap();
    assert_eq!(outcome, DeliveryFailureOutcome::WillRetry);

    let outcome = store
        .mark_delivery_failed(
            &event.event_id,
            "2026-07-05T00:00:20Z",
            "server returned 500",
            2,
        )
        .unwrap();
    assert_eq!(outcome, DeliveryFailureOutcome::Abandoned);

    let pending = store
        .pending_status_events("2026-07-05T00:00:21Z", 10)
        .unwrap();
    assert!(pending.is_empty());
}

#[test]
fn delivered_events_can_be_cleaned_by_retention_cutoff() {
    let store = store();
    let event = store
        .insert_status_event(&NewRemoteStatusEvent {
            job_id: "JOB-001",
            request_id: "REQ-001",
            batch_id: Some("BATCH-001"),
            status: RemoteReportStatus::Success,
            message: None,
            occurred_at: "2026-07-05T00:00:00Z",
            next_retry_at: "2026-07-05T00:00:00Z",
        })
        .unwrap()
        .unwrap();

    store
        .mark_delivered(&event.event_id, "2026-07-05T00:00:02Z")
        .unwrap();

    assert_eq!(
        store
            .cleanup_delivered_before("2026-07-05T00:00:03Z")
            .unwrap(),
        1
    );
}
