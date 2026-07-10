use crate::{
    app_state::AppState,
    config::AgentConfig,
    ip_whitelist::is_client_ip_allowed,
    logs::TaskLogEntry,
    printing::{PaperInfo, PrintError, PrinterInfo},
    protocol::{
        is_allowed_origin, ClientMessage, ErrorCode, JobStatus, JobValidationError,
        PrintQueueJobInfo, PrinterDetails, ServerMessage,
    },
    queue::QueueError,
    test_print::{print_calibration_page, TestPrintError},
};
use axum::{
    extract::{
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Request, State,
    },
    http::{
        header::{CONTENT_TYPE, ORIGIN},
        HeaderMap, HeaderValue, Method, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{AddrParseError, IpAddr, SocketAddr},
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
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            ip_whitelist_middleware,
        ))
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
pub async fn bind_listener(config: &AgentConfig) -> Result<(SocketAddr, TcpListener), ServerError> {
    let addr = configured_addr(config)?;
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    Ok((addr, listener))
}

/// 使用已经绑定的监听器运行本地服务。
pub async fn serve_listener(state: AppState, listener: TcpListener) -> Result<(), ServerError> {
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// 在当前任务中运行本地服务。
pub async fn run_server(state: AppState) -> Result<(), ServerError> {
    let config = state.config.read().await.clone();
    let (_, listener) = bind_listener(&config).await?;
    serve_listener(state, listener).await
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

/// 检查客户端 IP 是否被当前配置允许。
pub async fn is_client_ip_allowed_for_state(state: &AppState, client_ip: IpAddr) -> bool {
    let config = state.config.read().await;
    is_client_ip_allowed(client_ip, &config.security.allowed_ips)
}

/// 在所有 HTTP/WebSocket 路由前拦截未进入 IP 白名单的客户端。
async fn ip_whitelist_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(ConnectInfo(addr)) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
        return next.run(request).await;
    };

    if is_client_ip_allowed_for_state(&state, addr.ip()).await {
        next.run(request).await
    } else {
        client_ip_error_response()
    }
}

/// 构造客户端 IP 未进入白名单时的 HTTP 错误响应。
fn client_ip_error_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error_code: ErrorCode::OriginNotAllowed,
            message: "client ip is not allowed".to_string(),
        }),
    )
        .into_response()
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
        ClientMessage::GetPrintersList { request_id } => match state.printing.list_printers() {
            Ok(printers) => ClientTextOutcome::response(ServerMessage::PrintersList {
                request_id,
                printers,
            }),
            Err(error) => {
                ClientTextOutcome::response(print_error_response(Some(request_id), error))
            }
        },
        ClientMessage::GetPrinterInfo {
            request_id,
            printer_name,
        } => {
            let printer = match state.printing.list_printers() {
                Ok(printers) => printers
                    .into_iter()
                    .find(|printer| printer.name == printer_name),
                Err(error) => {
                    return ClientTextOutcome::response(print_error_response(
                        Some(request_id),
                        error,
                    ));
                }
            };

            let Some(printer) = printer else {
                return ClientTextOutcome::response(print_error_response(
                    Some(request_id),
                    PrintError::PrinterNotFound(printer_name),
                ));
            };

            let papers = match state.printing.list_papers(&printer.name) {
                Ok(papers) => papers,
                Err(error) => {
                    return ClientTextOutcome::response(print_error_response(
                        Some(request_id),
                        error,
                    ));
                }
            };
            let trays = match state.printing.list_trays(&printer.name) {
                Ok(trays) => trays,
                Err(error) => {
                    return ClientTextOutcome::response(print_error_response(
                        Some(request_id),
                        error,
                    ));
                }
            };
            let media_types = match state.printing.list_media_types(&printer.name) {
                Ok(media_types) => media_types,
                Err(error) => {
                    return ClientTextOutcome::response(print_error_response(
                        Some(request_id),
                        error,
                    ));
                }
            };

            ClientTextOutcome::response(ServerMessage::PrinterInfo {
                request_id,
                printer: PrinterDetails {
                    name: printer.name,
                    is_default: printer.is_default,
                    dpi: printer.dpi,
                    port: printer.port,
                    is_local: printer.is_local,
                    is_network: printer.is_network,
                    is_virtual: printer.is_virtual,
                    papers,
                    trays,
                    media_types,
                },
            })
        }
        ClientMessage::GetPrintQueue { request_id } => {
            let jobs = state
                .queue
                .lock()
                .await
                .pending_jobs()
                .into_iter()
                .map(|queued| PrintQueueJobInfo {
                    request_id: queued.request_id,
                    batch_id: queued.batch_id,
                    job_id: queued.job.job_id,
                    status: JobStatus::Queued,
                    message: Some("queued".to_string()),
                })
                .collect();

            ClientTextOutcome::response(ServerMessage::PrintQueue { request_id, jobs })
        }
        ClientMessage::Print { request_id, job } => {
            let max_file_size_mb = state.config.read().await.limits.max_file_size_mb;
            if let Err(error) = job.validate_for_acceptance(max_file_size_mb) {
                return ClientTextOutcome::response(job_validation_error_response(
                    Some(request_id),
                    error,
                ));
            }

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
            let max_file_size_mb = state.config.read().await.limits.max_file_size_mb;
            for job in &jobs {
                if let Err(error) = job.validate_for_acceptance(max_file_size_mb) {
                    return ClientTextOutcome::response(job_validation_error_response(
                        Some(request_id),
                        error,
                    ));
                }
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
        QueueError::InvalidMessage => ErrorCode::InvalidMessage,
    };

    ServerMessage::Error {
        request_id,
        error_code,
        message: error.to_string(),
    }
}

/// 把任务字段校验失败映射为 WebSocket 协议错误消息。
fn job_validation_error_response(
    request_id: Option<String>,
    error: JobValidationError,
) -> ServerMessage {
    let error_code = match error {
        JobValidationError::FileTooLarge => ErrorCode::FileTooLarge,
        JobValidationError::MissingRawData
        | JobValidationError::RawFileUrlNotAllowed
        | JobValidationError::RawPaperNotAllowed
        | JobValidationError::RawCopiesNotAllowed
        | JobValidationError::MissingFileUrl
        | JobValidationError::FileRawDataNotAllowed
        | JobValidationError::InvalidRawData
        | JobValidationError::MissingHtmlFileUrl
        | JobValidationError::InvalidHtmlFileUrl
        | JobValidationError::HtmlInlineNotAllowed
        | JobValidationError::MissingRawHtml
        | JobValidationError::RawHtmlFileUrlNotAllowed
        | JobValidationError::HtmlDataBase64NotAllowed
        | JobValidationError::NonHtmlHtmlNotAllowed
        | JobValidationError::NonHtmlWaitNotAllowed
        | JobValidationError::HtmlWaitOutOfRange => ErrorCode::InvalidMessage,
    };

    ServerMessage::Error {
        request_id,
        error_code,
        message: error.to_string(),
    }
}

/// 把打印后端错误映射为 WebSocket 协议错误消息。
fn print_error_response(request_id: Option<String>, error: PrintError) -> ServerMessage {
    let error_code = match error {
        PrintError::PrinterNotFound(_) => ErrorCode::PrinterNotFound,
        PrintError::PaperNotFound(_) => ErrorCode::PaperNotFound,
        PrintError::UnsupportedPlatform | PrintError::CommandFailed { .. } => {
            ErrorCode::PrintFailed
        }
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
        printing::{
            PaperInfo, PrintBackend, PrintOptions, PrintResult, PrintSubmission, PrinterInfo,
            RawPrintOptions,
        },
        protocol::{ErrorCode, JobStatus, JobValidationError, ServerMessage},
        queue::QueueError,
        server::{configured_addr, health, is_client_ip_allowed_for_state, is_ws_origin_allowed},
    };
    use axum::{
        body::{to_bytes, Body},
        extract::connect_info::ConnectInfo,
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
        net::{IpAddr, Ipv4Addr, SocketAddr},
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

    #[test]
    fn html_source_validation_errors_map_to_invalid_message() {
        assert!(matches!(
            super::job_validation_error_response(
                Some("request-1".to_string()),
                JobValidationError::InvalidHtmlFileUrl,
            ),
            ServerMessage::Error {
                error_code: ErrorCode::InvalidMessage,
                ..
            }
        ));
        assert!(matches!(
            super::queue_error_response(Some("request-1".to_string()), QueueError::InvalidMessage),
            ServerMessage::Error {
                error_code: ErrorCode::InvalidMessage,
                ..
            }
        ));
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

    #[test]
    fn job_status_message_serializes_submitted_status() {
        let json = serde_json::to_string(&ServerMessage::JobStatus {
            request_id: Some("request-1".to_string()),
            job_id: "job-1".to_string(),
            status: JobStatus::Submitted,
            message: Some("submitted to system print queue".to_string()),
        })
        .unwrap();

        assert!(json.contains(r#""status":"submitted""#));
        assert!(!json.contains(r#""status":"success""#));
    }

    #[tokio::test]
    async fn websocket_get_printers_list_returns_backend_printers() {
        let state = AppState::with_printing(
            AgentConfig::default(),
            Box::new(ListingPrintBackend {
                printers: vec![PrinterInfo {
                    name: "Zebra ZD421".to_string(),
                    is_default: true,
                    dpi: Some(203),
                    port: Some("usb://Zebra/ZD421".to_string()),
                    is_local: Some(true),
                    is_network: Some(false),
                    is_virtual: Some(false),
                }],
                papers: vec![],
                trays: vec![],
                media_types: vec![],
            }),
        );

        let outcome = super::handle_client_text(
            &state,
            r#"{"type":"get_printers_list","request_id":"REQ-PRINTERS"}"#,
        )
        .await;

        assert_eq!(
            outcome.response,
            ServerMessage::PrintersList {
                request_id: "REQ-PRINTERS".to_string(),
                printers: vec![PrinterInfo {
                    name: "Zebra ZD421".to_string(),
                    is_default: true,
                    dpi: Some(203),
                    port: Some("usb://Zebra/ZD421".to_string()),
                    is_local: Some(true),
                    is_network: Some(false),
                    is_virtual: Some(false),
                }],
            }
        );
    }

    #[tokio::test]
    async fn websocket_get_printer_info_returns_backend_papers() {
        let state = AppState::with_printing(
            AgentConfig::default(),
            Box::new(ListingPrintBackend {
                printers: vec![PrinterInfo {
                    name: "Zebra ZD421".to_string(),
                    is_default: true,
                    dpi: Some(203),
                    port: Some("usb://Zebra/ZD421".to_string()),
                    is_local: Some(true),
                    is_network: Some(false),
                    is_virtual: Some(false),
                }],
                papers: vec![PaperInfo {
                    id: "label_60x40".to_string(),
                    name: "60 x 40 mm".to_string(),
                    width_mm: 60.0,
                    height_mm: 40.0,
                }],
                trays: vec![crate::printing::PrinterTrayInfo {
                    id: "tray-1".to_string(),
                    name: "Tray 1".to_string(),
                }],
                media_types: vec![crate::printing::PrinterMediaTypeInfo {
                    id: "thermal-label".to_string(),
                    name: "Thermal Label".to_string(),
                }],
            }),
        );

        let outcome = super::handle_client_text(
            &state,
            r#"{"type":"get_printer_info","request_id":"REQ-INFO","printer_name":"Zebra ZD421"}"#,
        )
        .await;

        assert_eq!(
            outcome.response,
            ServerMessage::PrinterInfo {
                request_id: "REQ-INFO".to_string(),
                printer: super::PrinterDetails {
                    name: "Zebra ZD421".to_string(),
                    is_default: true,
                    dpi: Some(203),
                    port: Some("usb://Zebra/ZD421".to_string()),
                    is_local: Some(true),
                    is_network: Some(false),
                    is_virtual: Some(false),
                    papers: vec![PaperInfo {
                        id: "label_60x40".to_string(),
                        name: "60 x 40 mm".to_string(),
                        width_mm: 60.0,
                        height_mm: 40.0,
                    }],
                    trays: vec![crate::printing::PrinterTrayInfo {
                        id: "tray-1".to_string(),
                        name: "Tray 1".to_string(),
                    }],
                    media_types: vec![crate::printing::PrinterMediaTypeInfo {
                        id: "thermal-label".to_string(),
                        name: "Thermal Label".to_string(),
                    }],
                },
            }
        );
    }

    #[tokio::test]
    async fn websocket_get_print_queue_returns_pending_jobs() {
        let state = AppState::new(AgentConfig::default());
        state
            .queue
            .lock()
            .await
            .accept_job(
                "REQ-QUEUE-ITEM".to_string(),
                crate::protocol::PrintJobInput {
                    job_id: "JOB-QUEUE-ITEM".to_string(),
                    format: crate::protocol::SupportedFormat::Pdf,
                    printer_name: None,
                    file_url: Some("https://example.com/label.pdf".to_string()),
                    data_base64: None,
                    html: None,
                    wait_ms: None,
                    copies: Some(1),
                    paper: None,
                },
            )
            .unwrap();

        let outcome = super::handle_client_text(
            &state,
            r#"{"type":"get_print_queue","request_id":"REQ-QUEUE"}"#,
        )
        .await;

        assert_eq!(
            outcome.response,
            ServerMessage::PrintQueue {
                request_id: "REQ-QUEUE".to_string(),
                jobs: vec![super::PrintQueueJobInfo {
                    request_id: "REQ-QUEUE-ITEM".to_string(),
                    batch_id: None,
                    job_id: "JOB-QUEUE-ITEM".to_string(),
                    status: JobStatus::Queued,
                    message: Some("queued".to_string()),
                }],
            }
        );
    }

    struct ListingPrintBackend {
        printers: Vec<PrinterInfo>,
        papers: Vec<PaperInfo>,
        trays: Vec<crate::printing::PrinterTrayInfo>,
        media_types: Vec<crate::printing::PrinterMediaTypeInfo>,
    }

    impl PrintBackend for ListingPrintBackend {
        fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
            Ok(self.printers.clone())
        }

        fn list_papers(&self, _printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
            Ok(self.papers.clone())
        }

        fn list_trays(
            &self,
            _printer_name: &str,
        ) -> PrintResult<Vec<crate::printing::PrinterTrayInfo>> {
            Ok(self.trays.clone())
        }

        fn list_media_types(
            &self,
            _printer_name: &str,
        ) -> PrintResult<Vec<crate::printing::PrinterMediaTypeInfo>> {
            Ok(self.media_types.clone())
        }

        fn print_pdf(&self, _path: &Path, _options: &PrintOptions) -> PrintResult<PrintSubmission> {
            Ok(mock_submission())
        }

        fn print_raw(
            &self,
            _data: &[u8],
            _options: &RawPrintOptions,
        ) -> PrintResult<PrintSubmission> {
            Ok(mock_submission())
        }
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

        fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<PrintSubmission> {
            self.calls.lock().unwrap().push(PrintCall {
                path: path.to_path_buf(),
                path_bytes: fs::read(path).unwrap(),
                options: options.clone(),
            });
            Ok(mock_submission())
        }

        fn print_raw(
            &self,
            _data: &[u8],
            _options: &RawPrintOptions,
        ) -> PrintResult<PrintSubmission> {
            Ok(mock_submission())
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

    #[tokio::test]
    async fn ws_origin_gate_uses_configured_allowed_origins() {
        let config = AgentConfig {
            security: SecurityConfig {
                allowed_origins: vec!["http://localhost:5173".to_string()],
                allowed_ips: vec!["127.0.0.1".to_string()],
            },
            ..AgentConfig::default()
        };
        let state = AppState::new(config);

        assert!(is_ws_origin_allowed(&state, Some("http://localhost:5173")).await);
        assert!(!is_ws_origin_allowed(&state, Some("https://evil.example")).await);
        assert!(!is_ws_origin_allowed(&state, None).await);
    }

    #[tokio::test]
    async fn client_ip_gate_allows_loopback_even_when_missing_from_config() {
        let config = AgentConfig {
            security: SecurityConfig {
                allowed_origins: Vec::new(),
                allowed_ips: Vec::new(),
            },
            ..AgentConfig::default()
        };
        let state = AppState::new(config);

        assert!(
            is_client_ip_allowed_for_state(&state, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))).await
        );
    }

    #[tokio::test]
    async fn client_ip_gate_uses_single_ip_and_cidr_entries() {
        let config = AgentConfig {
            security: SecurityConfig {
                allowed_origins: Vec::new(),
                allowed_ips: vec![
                    "127.0.0.1".to_string(),
                    "192.168.1.0/24".to_string(),
                    "10.0.0.8".to_string(),
                ],
            },
            ..AgentConfig::default()
        };
        let state = AppState::new(config);

        assert!(
            is_client_ip_allowed_for_state(&state, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)))
                .await
        );
        assert!(
            is_client_ip_allowed_for_state(&state, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8))).await
        );
        assert!(
            !is_client_ip_allowed_for_state(&state, IpAddr::V4(Ipv4Addr::new(192, 168, 2, 20)))
                .await
        );
    }

    #[tokio::test]
    async fn http_routes_reject_disallowed_client_ip() {
        let config = AgentConfig {
            security: SecurityConfig {
                allowed_origins: Vec::new(),
                allowed_ips: vec!["127.0.0.1".to_string()],
            },
            ..AgentConfig::default()
        };
        let app = super::router(AppState::new(config));
        let mut request = Request::builder()
            .method(Method::GET)
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 20], 50000))));

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
