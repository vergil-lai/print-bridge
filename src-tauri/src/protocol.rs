use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;
use url::Url;

/// 浏览器侧打印任务可提交的文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportedFormat {
    Pdf,
    Image,
    Png,
    Jpg,
    Jpeg,
}

/// 字符串无法解析为支持格式时返回的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSupportedFormatError;

impl fmt::Display for ParseSupportedFormatError {
    /// 写入稳定的可读解析错误。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unsupported file format")
    }
}

impl std::error::Error for ParseSupportedFormatError {}

impl FromStr for SupportedFormat {
    type Err = ParseSupportedFormatError;

    /// 解析浏览器 SDK 使用的小写传输格式名称。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pdf" => Ok(Self::Pdf),
            "image" => Ok(Self::Image),
            "png" => Ok(Self::Png),
            "jpg" => Ok(Self::Jpg),
            "jpeg" => Ok(Self::Jpeg),
            _ => Err(ParseSupportedFormatError),
        }
    }
}

/// 浏览器 Origin、文件 URL 和纸张尺寸的校验错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid origin")]
    InvalidOrigin,
    #[error("unsupported origin scheme")]
    UnsupportedOriginScheme,
    #[error("invalid file url")]
    InvalidFileUrl,
    #[error("unsupported file url scheme")]
    UnsupportedFileUrlScheme,
    #[error("paper dimensions must be positive")]
    InvalidPaperDimensions,
}

/// 任务和配置合并后的纸张尺寸。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectivePaper {
    pub width_mm: f64,
    pub height_mm: f64,
}

impl EffectivePaper {
    /// 确保纸张宽高都为正数。
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.width_mm > 0.0 && self.height_mm > 0.0 {
            Ok(())
        } else {
            Err(ProtocolError::InvalidPaperDimensions)
        }
    }
}

/// 从浏览器客户端收到的单个打印任务。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintJobInput {
    pub job_id: String,
    pub format: SupportedFormat,
    pub file_url: String,
    #[serde(default = "default_copies")]
    pub copies: u16,
    #[serde(default)]
    pub paper: Option<EffectivePaper>,
}

/// 打印任务未提供份数字段时使用的默认值。
fn default_copies() -> u16 {
    1
}

/// WebSocket 协议接受的浏览器客户端消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "ping")]
    Ping { time: i64 },
    #[serde(rename = "print")]
    Print {
        request_id: String,
        #[serde(flatten)]
        job: PrintJobInput,
    },
    #[serde(rename = "print_batch")]
    PrintBatch {
        request_id: String,
        batch_id: String,
        jobs: Vec<PrintJobInput>,
    },
}

/// 本地 Agent 返回给浏览器客户端的消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "pong")]
    Pong { time: i64, agent_status: String },
    #[serde(rename = "job_status")]
    JobStatus {
        request_id: Option<String>,
        job_id: String,
        status: JobStatus,
        message: Option<String>,
    },
    #[serde(rename = "error")]
    Error {
        request_id: Option<String>,
        error_code: ErrorCode,
        message: String,
    },
}

/// 协议和本地日志中暴露的打印任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Downloading,
    Printing,
    Submitted,
    Completed,
    Failed,
    Unknown,
    Cancelled,
}

/// 以 SCREAMING_SNAKE_CASE 字符串序列化的协议错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    OriginNotAllowed,
    InvalidMessage,
    PrinterNotConfigured,
    PrinterNotFound,
    PaperNotConfigured,
    PaperNotFound,
    DownloadFailed,
    FileTooLarge,
    UnsupportedFormat,
    FormatMismatch,
    PrintFailed,
    JobDuplicated,
    BatchDuplicated,
    BatchTooLarge,
    CopiesOutOfRange,
    ServicePortInUse,
    InternalError,
}

/// 检查传入 WebSocket Origin 是否精确匹配允许列表。
pub fn is_allowed_origin(origin: Option<&str>, allowed_origins: &[String]) -> bool {
    origin.is_some_and(|origin| allowed_origins.iter().any(|allowed| allowed == origin))
}

/// 校验并规范化允许的浏览器 Origin 字符串。
pub fn validate_origin(value: &str) -> Result<(), ProtocolError> {
    let url = Url::parse(value).map_err(|_| ProtocolError::InvalidOrigin)?;

    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(ProtocolError::UnsupportedOriginScheme),
    }

    if url.origin().ascii_serialization() == value {
        Ok(())
    } else {
        Err(ProtocolError::InvalidOrigin)
    }
}

/// 校验打印文件 URL 是否可通过 HTTP(S) 下载或以内联 PDF data URL 提供。
pub fn validate_file_url(value: &str) -> Result<Url, ProtocolError> {
    let url = Url::parse(value).map_err(|_| ProtocolError::InvalidFileUrl)?;

    match url.scheme() {
        "http" | "https" => Ok(url),
        "data" if is_pdf_data_url(value) => Ok(url),
        _ => Err(ProtocolError::UnsupportedFileUrlScheme),
    }
}

/// 判断 file_url 是否是 PrintBridge 接受的 base64 PDF data URL。
pub fn is_pdf_data_url(value: &str) -> bool {
    let Some(payload) = value.strip_prefix("data:") else {
        return false;
    };
    let Some((metadata, data)) = payload.split_once(',') else {
        return false;
    };
    if data.is_empty() {
        return false;
    }

    let mut parts = metadata.split(';');
    let Some(media_type) = parts.next() else {
        return false;
    };

    media_type.eq_ignore_ascii_case("application/pdf")
        && parts.any(|part| part.eq_ignore_ascii_case("base64"))
}
