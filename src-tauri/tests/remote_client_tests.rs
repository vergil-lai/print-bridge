use print_bridge_lib::{
    config::RemoteConfig,
    remote_client::{RemoteClient, RemoteClientError},
    remote_store::{RemoteDeliveryState, RemoteReportStatus, RemoteStatusEvent},
};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[tokio::test]
async fn fetch_tasks_sends_auth_and_device_headers() {
    let server = TestServer::start(response(
        "200 OK",
        r#"{"type":"print","request_id":"REQ-001","job_id":"JOB-001","format":"pdf","file_url":"https://example.com/label.pdf","copies":1}"#,
    ));

    let tasks = RemoteClient::default()
        .fetch_tasks(&config(server.url()))
        .await
        .unwrap();
    let request = server.request();

    assert_eq!(tasks.len(), 1);
    assert!(request.starts_with("get /print-task http/1.1"), "{request}");
    assert!(request.contains("authorization: bearer secret-token"));
    assert!(request.contains("x-printbridge-device-id: device-001"));
    assert!(request.contains("x-printbridge-device-name: packing-station"));
}

#[tokio::test]
async fn report_status_posts_event_and_accepts_any_2xx_status() {
    let server = TestServer::start(response("204 No Content", ""));
    let event = status_event(RemoteReportStatus::Success);

    RemoteClient::default()
        .report_status(&config(server.url()), &event)
        .await
        .unwrap();
    let request = server.request();

    assert!(
        request.starts_with("post /print-task http/1.1"),
        "{request}"
    );
    assert!(request.contains("authorization: bearer secret-token"));
    assert!(request.contains(r#""event":"status""#));
    assert!(request.contains(r#""event_id":"event-001""#));
    assert!(request.contains(r#""status":"success""#));
    assert!(request.contains(r#""device_id":"device-001""#));
    assert!(request.contains(r#""device_name":"packing-station""#));
}

#[tokio::test]
async fn report_status_rejects_non_2xx_status() {
    let server = TestServer::start(response("500 Internal Server Error", "nope"));
    let event = status_event(RemoteReportStatus::Failed);

    let result = RemoteClient::default()
        .report_status(&config(server.url()), &event)
        .await;

    assert!(matches!(result, Err(RemoteClientError::HttpStatus(500))));
}

fn config(endpoint_url: String) -> RemoteConfig {
    RemoteConfig {
        enabled: true,
        endpoint_url: Some(endpoint_url),
        bearer_token: Some("secret-token".to_string()),
        device_id: Some("device-001".to_string()),
        device_name: Some("packing-station".to_string()),
        poll_interval_seconds: 10,
        max_report_retries: 10,
        history_retention_days: 3,
    }
}

fn status_event(status: RemoteReportStatus) -> RemoteStatusEvent {
    RemoteStatusEvent {
        event_id: "EVENT-001".to_string(),
        job_id: "JOB-001".to_string(),
        request_id: "REQ-001".to_string(),
        batch_id: None,
        status,
        message: None,
        occurred_at: "2026-07-05T00:00:00Z".to_string(),
        delivery_state: RemoteDeliveryState::Pending,
        retry_count: 0,
        next_retry_at: "2026-07-05T00:00:00Z".to_string(),
        last_error: None,
    }
}

struct TestServer {
    address: SocketAddr,
    handle: Option<thread::JoinHandle<()>>,
    request: Arc<Mutex<Option<String>>>,
}

impl TestServer {
    fn start(response: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let request = Arc::new(Mutex::new(None));
        let request_for_thread = request.clone();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            *request_for_thread.lock().unwrap() = Some(request);
            stream.write_all(response.as_bytes()).unwrap();
        });

        Self {
            address,
            handle: Some(handle),
            request,
        }
    }

    fn url(&self) -> String {
        format!("http://{}/print-task", self.address)
    }

    fn request(&self) -> String {
        self.request.lock().unwrap().clone().unwrap_or_default()
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
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(header_end) = find_header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let body_read = bytes.len().saturating_sub(header_end + 4);
            if body_read < content_length {
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.len().saturating_sub(header_end + 4) >= content_length {
                        break;
                    }
                }
            }
            break;
        }
    }

    String::from_utf8_lossy(&bytes).to_lowercase()
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
