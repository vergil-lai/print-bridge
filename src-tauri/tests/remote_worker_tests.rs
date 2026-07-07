use print_bridge_lib::{
    app_state::AppState,
    config::{AgentConfig, RemoteConfig},
    printing::default_backend,
    remote_client::RemoteClient,
    remote_store::{NewRemoteStatusEvent, RemoteReportStatus, RemoteStore},
    remote_worker::{poll_once, report_pending_once},
};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::Duration,
};

#[tokio::test]
async fn poll_once_enqueues_new_remote_jobs() {
    let server = TestServer::start(response(
        "200 OK",
        r#"{
        "type": "print",
        "request_id": "REQ-001",
        "job_id": "JOB-001",
        "format": "pdf",
        "file_url": "https://example.com/label.pdf",
        "copies": 1
    }"#,
    ));
    let state = state(server.url());

    let outcome = poll_once(&state, &RemoteClient::default()).await.unwrap();
    let queued = state.queue.lock().await.pop_next().unwrap();

    assert_eq!(outcome.enqueued, 1);
    assert!(queued.remote);
    assert_eq!(queued.request_id, "REQ-001");
    assert_eq!(queued.job.job_id, "JOB-001");
}

#[tokio::test]
async fn report_pending_once_marks_2xx_events_delivered() {
    let server = TestServer::start(response("204 No Content", ""));
    let state = state(server.url());
    let store = state.remote_store.as_ref().unwrap();
    store
        .insert_status_event(&NewRemoteStatusEvent {
            job_id: "JOB-001",
            request_id: "REQ-001",
            batch_id: None,
            status: RemoteReportStatus::Success,
            message: None,
            occurred_at: "1970-01-01T00:00:00Z",
            next_retry_at: "1970-01-01T00:00:00Z",
        })
        .unwrap();

    let outcome = report_pending_once(&state, &RemoteClient::default())
        .await
        .unwrap();

    assert_eq!(outcome.delivered, 1);
    assert!(store
        .pending_status_events("9999-01-01T00:00:00Z", 10)
        .unwrap()
        .is_empty());
}

fn state(endpoint_url: String) -> AppState {
    let config = AgentConfig {
        remote: RemoteConfig {
            enabled: true,
            endpoint_url: Some(endpoint_url),
            bearer_token: None,
            device_id: None,
            device_name: None,
            poll_interval_seconds: 10,
            max_report_retries: 10,
            history_retention_days: 3,
        },
        ..AgentConfig::default()
    };

    AppState::with_printing(config, default_backend())
        .with_remote_store(RemoteStore::open_in_memory().unwrap())
}

struct TestServer {
    address: SocketAddr,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(response: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            stream.write_all(response.as_bytes()).unwrap();
        });

        Self {
            address,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/print-task", self.address)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(header_end) = find_header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while bytes.len().saturating_sub(header_end + 4) < content_length {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            break;
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
