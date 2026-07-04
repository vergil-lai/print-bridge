use crate::{
    app_state::AppState,
    config::AgentConfig,
    logs::TaskLogEntry,
    printing::{PaperInfo, PrintError, PrinterInfo},
    protocol::{is_allowed_origin, ClientMessage, ErrorCode, JobStatus, ServerMessage},
    queue::QueueError,
    test_print::{print_calibration_page, TestPrintError},
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{
        header::{CONTENT_TYPE, ORIGIN},
        HeaderMap, HeaderValue, Method, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{AddrParseError, SocketAddr},
    str::FromStr,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

const SERVICE_NAME: &str = "print-bridge";
const BIND_HOST: &str = "0.0.0.0";

/// 绑定或启动本地服务时可能出现的错误。
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("server bind failed: {0}")]
    Bind(#[from] std::io::Error),
    #[error("invalid server address: {0}")]
    InvalidAddress(#[from] AddrParseError),
}

/// UI 和外部诊断使用的健康检查响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
}

/// 纸张列表响应，包含是否支持自定义纸张尺寸。
#[derive(Debug, Serialize)]
struct PapersResponse {
    papers: Vec<PaperInfo>,
    supports_custom: bool,
}

/// HTTP 接口返回的 JSON 错误体。
#[derive(Debug, Serialize, Deserialize)]
struct ErrorResponse {
    error_code: ErrorCode,
    message: String,
}

/// 构建 Agent 的本地 HTTP 和 WebSocket 路由。
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_route))
        .route("/printers", get(list_printers))
        .route("/printers/{printer_name}/papers", get(list_papers))
        .route("/config", get(get_config).post(update_config))
        .route("/logs", get(get_logs))
        .route("/print/test", post(print_test))
        .route("/ws", get(ws_handler))
        .with_state(state)
        .layer(settings_cors_layer())
}

/// 把浏览器 CORS 限制在桌面设置 UI 的 Origin 内。
fn settings_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("tauri://localhost"),
            HeaderValue::from_static("http://tauri.localhost"),
        ])
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([CONTENT_TYPE])
}

/// 解析服务绑定地址。Agent 固定监听所有网卡，供局域网客户端连接。
pub fn configured_addr(config: &AgentConfig) -> Result<SocketAddr, AddrParseError> {
    SocketAddr::from_str(&format!("{}:{}", BIND_HOST, config.service.port))
}

/// 在当前任务中运行本地服务。
pub async fn run_server(state: AppState) -> Result<(), ServerError> {
    let config = state.config.read().await.clone();
    let addr = configured_addr(&config)?;
    let listener = TcpListener::bind(addr).await?;

    axum::serve(listener, router(state)).await?;
    Ok(())
}

/// 返回服务健康检查内容。
pub async fn health() -> HealthResponse {
    HealthResponse {
        status: "ok",
        service: SERVICE_NAME,
    }
}

/// 健康检查响应的 Axum 路由包装。
async fn health_route() -> Json<HealthResponse> {
    Json(health().await)
}

/// 从当前打印后端读取打印机列表。
async fn list_printers(State(state): State<AppState>) -> Result<Json<Vec<PrinterInfo>>, ApiError> {
    state
        .printing
        .list_printers()
        .map(Json)
        .map_err(ApiError::from)
}

/// 读取指定打印机的已知纸张或兜底纸张尺寸。
async fn list_papers(
    State(state): State<AppState>,
    Path(printer_name): Path<String>,
) -> Result<Json<PapersResponse>, ApiError> {
    state
        .printing
        .list_papers(&printer_name)
        .map(|papers| {
            Json(PapersResponse {
                papers,
                supports_custom: true,
            })
        })
        .map_err(ApiError::from)
}

/// 返回当前内存中的 Agent 配置。
async fn get_config(State(state): State<AppState>) -> Json<AgentConfig> {
    Json(state.config.read().await.clone())
}

/// 校验、持久化并应用新的 Agent 配置。
async fn update_config(
    State(state): State<AppState>,
    Json(config): Json<AgentConfig>,
) -> Result<Json<AgentConfig>, ApiError> {
    state
        .save_config(config)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

/// 返回设置 UI 需要的最近任务日志。
async fn get_logs(State(state): State<AppState>) -> Json<Vec<TaskLogEntry>> {
    Json(state.logs.lock().await.recent())
}

/// 使用当前默认打印设置提交标签校准测试页。
async fn print_test(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    if !is_settings_origin_allowed(headers.get(ORIGIN).and_then(|value| value.to_str().ok())) {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            error_code: ErrorCode::OriginNotAllowed,
            message: "origin is not allowed".to_string(),
        });
    }

    print_calibration_page(&state)
        .await
        .map(|()| StatusCode::ACCEPTED)
        .map_err(ApiError::from)
}

/// 把允许的浏览器连接升级为 PrintBridge WebSocket 协议。
async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    if !is_ws_origin_allowed(&state, origin).await {
        return StatusCode::FORBIDDEN.into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(state, socket))
}

/// 检查 WebSocket 请求 Origin 是否被当前配置允许。
pub async fn is_ws_origin_allowed(state: &AppState, origin: Option<&str>) -> bool {
    let config = state.config.read().await;
    is_allowed_origin(origin, &config.security.allowed_origins)
}

/// 检查仅供桌面设置 UI 调用的 HTTP API Origin。
fn is_settings_origin_allowed(origin: Option<&str>) -> bool {
    matches!(
        origin,
        Some("http://localhost:1420" | "tauri://localhost" | "http://tauri.localhost")
    )
}

/// 处理单个 WebSocket 连接，并只转发该连接接受的任务。
async fn handle_socket(state: AppState, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let mut status_events = state.subscribe_status_events();
    // 每个浏览器连接只接收自己提交任务的状态事件。
    let mut accepted_job_ids = HashSet::new();

    loop {
        tokio::select! {
            result = receiver.next() => {
                let Some(result) = result else {
                    break;
                };
                let message = match result {
                    Ok(message) => message,
                    Err(error) => {
                        tauri_plugin_log::log::debug!("websocket receive failed: {error}");
                        break;
                    }
                };

                let outcome = match message {
                    Message::Text(text) => handle_client_text(&state, &text).await,
                    Message::Ping(payload) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    Message::Close(_) => break,
                    _ => continue,
                };

                accepted_job_ids.extend(outcome.accepted_job_ids);
                match serde_json::to_string(&outcome.response) {
                    Ok(json) => {
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        tauri_plugin_log::log::error!(
                            "websocket response serialization failed: {error}"
                        );
                        break;
                    }
                }
            }
            event = status_events.recv() => {
                match event {
                    Ok(entry) => {
                        let Some(response) = status_message_for_connection(&entry, &accepted_job_ids) else {
                            continue;
                        };
                        match serde_json::to_string(&response) {
                            Ok(json) => {
                                if sender.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                tauri_plugin_log::log::error!(
                                    "websocket status serialization failed: {error}"
                                );
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tauri_plugin_log::log::debug!(
                            "websocket status receiver skipped {skipped} events"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// 单条客户端消息的处理结果，包含新绑定到该 socket 的任务。
struct ClientTextOutcome {
    response: ServerMessage,
    accepted_job_ids: Vec<String>,
}

impl ClientTextOutcome {
    /// 构造未接受新任务的响应结果。
    fn response(response: ServerMessage) -> Self {
        Self {
            response,
            accepted_job_ids: Vec::new(),
        }
    }
}

/// 解析单条客户端文本帧，并返回协议响应。
async fn handle_client_text(state: &AppState, text: &str) -> ClientTextOutcome {
    let message = match serde_json::from_str::<ClientMessage>(text) {
        Ok(message) => message,
        Err(error) => {
            return ClientTextOutcome::response(ServerMessage::Error {
                request_id: None,
                error_code: ErrorCode::InvalidMessage,
                message: error.to_string(),
            });
        }
    };

    match message {
        ClientMessage::Ping { time } => ClientTextOutcome::response(ServerMessage::Pong {
            time,
            agent_status: "ready".to_string(),
        }),
        ClientMessage::Print { request_id, job } => {
            let job_id = job.job_id.clone();
            let result = state.queue.lock().await.accept_job(request_id.clone(), job);
            match result {
                Ok(()) => {
                    state.queue_notify.notify_one();
                    ClientTextOutcome {
                        accepted_job_ids: vec![job_id.clone()],
                        response: ServerMessage::JobStatus {
                            request_id: Some(request_id),
                            job_id,
                            status: JobStatus::Queued,
                            message: Some("queued".to_string()),
                        },
                    }
                }
                Err(error) => {
                    ClientTextOutcome::response(queue_error_response(Some(request_id), error))
                }
            }
        }
        ClientMessage::PrintBatch {
            request_id,
            batch_id,
            jobs,
        } => {
            let max_batch_jobs = state.config.read().await.limits.max_batch_jobs;
            if jobs.len() > max_batch_jobs {
                return ClientTextOutcome::response(ServerMessage::Error {
                    request_id: Some(request_id),
                    error_code: ErrorCode::BatchTooLarge,
                    message: "batch contains too many jobs".to_string(),
                });
            }

            // 保存所有已接受的任务 ID，便于后续 worker 状态广播
            // 只过滤回当前这个 WebSocket 连接。
            let queued = jobs.len();
            let job_ids = jobs
                .iter()
                .map(|job| job.job_id.clone())
                .collect::<Vec<_>>();
            let response_job_id = batch_id.clone();
            let result = state
                .queue
                .lock()
                .await
                .accept_batch(request_id.clone(), batch_id, jobs);
            match result {
                Ok(()) => {
                    state.queue_notify.notify_one();
                    ClientTextOutcome {
                        accepted_job_ids: job_ids,
                        response: ServerMessage::JobStatus {
                            request_id: Some(request_id),
                            job_id: response_job_id,
                            status: JobStatus::Queued,
                            message: Some(format!("batch accepted: {queued} jobs queued")),
                        },
                    }
                }
                Err(error) => {
                    ClientTextOutcome::response(queue_error_response(Some(request_id), error))
                }
            }
        }
    }
}

/// 把任务日志记录转换为单个连接的 WebSocket 状态消息。
fn status_message_for_connection(
    entry: &TaskLogEntry,
    accepted_job_ids: &HashSet<String>,
) -> Option<ServerMessage> {
    let job_id = entry.job_id.as_ref()?;
    if !accepted_job_ids.contains(job_id) {
        return None;
    }

    Some(ServerMessage::JobStatus {
        request_id: entry.request_id.clone(),
        job_id: job_id.clone(),
        status: entry.status,
        message: Some(entry.message.clone()),
    })
}

/// 把队列接收失败映射为协议错误消息。
fn queue_error_response(request_id: Option<String>, error: QueueError) -> ServerMessage {
    let error_code = match error {
        QueueError::DuplicateJobId => ErrorCode::JobDuplicated,
        QueueError::DuplicateBatchId => ErrorCode::BatchDuplicated,
    };

    ServerMessage::Error {
        request_id,
        error_code,
        message: error.to_string(),
    }
}

/// 同时携带 HTTP 状态码和协议错误码的错误包装。
struct ApiError {
    status: StatusCode,
    error_code: ErrorCode,
    message: String,
}

impl From<PrintError> for ApiError {
    /// 把打印后端错误转换为 HTTP 响应。
    fn from(error: PrintError) -> Self {
        let (status, error_code) = match error {
            PrintError::PrinterNotFound(_) => (StatusCode::NOT_FOUND, ErrorCode::PrinterNotFound),
            PrintError::PaperNotFound(_) => (StatusCode::NOT_FOUND, ErrorCode::PaperNotFound),
            PrintError::UnsupportedPlatform | PrintError::CommandFailed { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::PrintFailed)
            }
        };

        Self {
            status,
            error_code,
            message: error.to_string(),
        }
    }
}

impl From<TestPrintError> for ApiError {
    /// 把测试打印错误转换为 HTTP 响应。
    fn from(error: TestPrintError) -> Self {
        match error {
            TestPrintError::PrinterNotConfigured => Self {
                status: StatusCode::BAD_REQUEST,
                error_code: ErrorCode::PrinterNotConfigured,
                message: error.to_string(),
            },
            TestPrintError::PaperNotConfigured => Self {
                status: StatusCode::BAD_REQUEST,
                error_code: ErrorCode::PaperNotConfigured,
                message: error.to_string(),
            },
            TestPrintError::Document(_) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                error_code: ErrorCode::InternalError,
                message: error.to_string(),
            },
            TestPrintError::Print(error) => Self::from(error),
        }
    }
}

impl ApiError {
    /// 构造通用内部服务错误响应。
    fn internal(error: impl ToString) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error_code: ErrorCode::InternalError,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    /// 把 API 错误序列化为 Axum 响应类型。
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error_code: self.error_code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app_state::AppState,
        config::{AgentConfig, PrintingConfig, SecurityConfig, ServiceConfig},
        logs::TaskLogEntry,
        printing::{PaperInfo, PrintBackend, PrintOptions, PrintResult, PrinterInfo},
        protocol::{ErrorCode, JobStatus, ServerMessage},
        server::{configured_addr, health, is_ws_origin_allowed},
    };
    use axum::{
        body::{to_bytes, Body},
        http::{
            header::{
                ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_METHOD, CONTENT_TYPE, ORIGIN,
            },
            Method, Request, StatusCode,
        },
    };
    use std::{
        collections::HashSet,
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_reports_service_status() {
        let response = health().await;

        assert_eq!(response.status, "ok");
        assert_eq!(response.service, "print-bridge");
    }

    #[test]
    fn router_builds_with_app_state() {
        let _router = super::router(AppState::new(AgentConfig::default()));
    }

    #[tokio::test]
    async fn local_settings_origins_can_preflight_http_api() {
        let cases = [
            ("http://localhost:1420", Method::GET, "/printers"),
            ("tauri://localhost", Method::GET, "/printers"),
            ("http://tauri.localhost", Method::POST, "/config"),
        ];

        for (origin, requested_method, uri) in cases {
            let response = super::router(AppState::new(AgentConfig::default()))
                .oneshot(
                    Request::builder()
                        .method(Method::OPTIONS)
                        .uri(uri)
                        .header(ORIGIN, origin)
                        .header(ACCESS_CONTROL_REQUEST_METHOD, requested_method.as_str())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
                origin,
            );
        }
    }

    #[test]
    fn configured_addr_binds_all_interfaces_and_uses_configured_port() {
        let config = AgentConfig {
            service: ServiceConfig {
                host: "127.0.0.1".to_string(),
                port: 19001,
            },
            ..AgentConfig::default()
        };

        let addr = configured_addr(&config).unwrap();

        assert_eq!(addr.to_string(), "0.0.0.0:19001");
    }

    #[tokio::test]
    async fn http_config_post_persists_normalized_config() {
        let path = std::env::temp_dir().join(format!(
            "print-bridge-http-config-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let state = AppState::with_config_path(AgentConfig::default(), path.clone());
        let app = super::router(state);
        let mut config = AgentConfig::default();
        config.service.host = "0.0.0.0".to_string();
        config.service.port = 19191;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/config")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let returned: AgentConfig = serde_json::from_slice(&body).unwrap();
        assert_eq!(returned.service.host, "127.0.0.1");
        assert_eq!(returned.service.port, 19191);
        let loaded = AgentConfig::load(&path).unwrap();
        assert_eq!(loaded.service.host, "127.0.0.1");
        assert_eq!(loaded.service.port, 19191);

        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn print_test_requires_default_printer_and_paper() {
        let response = super::router(AppState::new(AgentConfig::default()))
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/print/test")
                    .header(ORIGIN, "tauri://localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: super::ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.error_code, ErrorCode::PrinterNotConfigured);
        assert!(error.message.contains("printer not configured"));
    }

    #[tokio::test]
    async fn print_test_requires_default_paper() {
        let state = AppState::with_printing(
            AgentConfig {
                printing: PrintingConfig {
                    default_printer: Some("Printer A".to_string()),
                    default_paper: None,
                    default_copies: 1,
                },
                ..AgentConfig::default()
            },
            Box::new(MockPrintBackend {
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
        );

        let response = super::router(state)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/print/test")
                    .header(ORIGIN, "tauri://localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: super::ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.error_code, ErrorCode::PaperNotConfigured);
        assert!(error.message.contains("paper not configured"));
    }

    #[tokio::test]
    async fn print_test_rejects_untrusted_origin() {
        let response = super::router(AppState::new(AgentConfig::default()))
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/print/test")
                    .header(ORIGIN, "https://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: super::ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.error_code, ErrorCode::OriginNotAllowed);
    }

    #[tokio::test]
    async fn print_test_generates_calibration_pdf_for_default_paper() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let state = AppState::with_printing(
            AgentConfig {
                printing: PrintingConfig {
                    default_printer: Some("Printer A".to_string()),
                    default_paper: Some(crate::protocol::EffectivePaper {
                        width_mm: 60.0,
                        height_mm: 40.0,
                    }),
                    default_copies: 1,
                },
                ..AgentConfig::default()
            },
            Box::new(MockPrintBackend {
                calls: calls.clone(),
            }),
        );

        let response = super::router(state)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/print/test")
                    .header(ORIGIN, "tauri://localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.options.printer_name, "Printer A");
        assert_eq!(call.options.paper.width_mm, 60.0);
        assert_eq!(call.options.paper.height_mm, 40.0);
        assert_eq!(call.options.copies, 1);
        assert_eq!(
            call.path.extension().and_then(|value| value.to_str()),
            Some("pdf")
        );
        assert!(call.path_bytes.starts_with(b"%PDF-"));
        assert!(call
            .path_bytes
            .windows(b"PrintBridge Test".len())
            .any(|window| window == b"PrintBridge Test"));
    }

    #[test]
    fn status_event_for_connection_filters_unaccepted_jobs() {
        let mut accepted = HashSet::new();
        accepted.insert("job-1".to_string());
        let matching = TaskLogEntry {
            timestamp: "2026-07-04T00:00:00Z".to_string(),
            request_id: Some("request-1".to_string()),
            batch_id: None,
            job_id: Some("job-1".to_string()),
            origin: None,
            status: JobStatus::Printing,
            message: "printing".to_string(),
        };
        let other = TaskLogEntry {
            job_id: Some("job-2".to_string()),
            ..matching.clone()
        };

        assert_eq!(
            super::status_message_for_connection(&matching, &accepted),
            Some(ServerMessage::JobStatus {
                request_id: Some("request-1".to_string()),
                job_id: "job-1".to_string(),
                status: JobStatus::Printing,
                message: Some("printing".to_string()),
            })
        );
        assert_eq!(
            super::status_message_for_connection(&other, &accepted),
            None
        );
    }

    struct MockPrintBackend {
        calls: Arc<Mutex<Vec<PrintCall>>>,
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

        fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<()> {
            self.calls.lock().unwrap().push(PrintCall {
                path: path.to_path_buf(),
                path_bytes: fs::read(path).unwrap(),
                options: options.clone(),
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn ws_origin_gate_uses_configured_allowed_origins() {
        let config = AgentConfig {
            security: SecurityConfig {
                allowed_origins: vec!["http://localhost:5173".to_string()],
            },
            ..AgentConfig::default()
        };
        let state = AppState::new(config);

        assert!(is_ws_origin_allowed(&state, Some("http://localhost:5173")).await);
        assert!(!is_ws_origin_allowed(&state, Some("https://evil.example")).await);
        assert!(!is_ws_origin_allowed(&state, None).await);
    }
}
