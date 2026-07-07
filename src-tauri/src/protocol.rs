use crate::printing::{PaperInfo, PrinterInfo, PrinterMediaTypeInfo, PrinterTrayInfo};
use base64::{engine::general_purpose::STANDARD, Engine as _};
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
    Docx,
    Xlsx,
    Pptx,
    Raw,
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
            "docx" => Ok(Self::Docx),
            "xlsx" => Ok(Self::Xlsx),
            "pptx" => Ok(Self::Pptx),
            "raw" => Ok(Self::Raw),
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

/// 单个打印任务字段组合不符合协议时返回的错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobValidationError {
    #[error("raw job requires data_base64")]
    MissingRawData,
    #[error("raw job does not accept file_url")]
    RawFileUrlNotAllowed,
    #[error("raw job does not accept paper")]
    RawPaperNotAllowed,
    #[error("raw job does not accept copies")]
    RawCopiesNotAllowed,
    #[error("file job requires file_url")]
    MissingFileUrl,
    #[error("file job does not accept data_base64")]
    FileRawDataNotAllowed,
    #[error("invalid raw data_base64")]
    InvalidRawData,
    #[error("file too large")]
    FileTooLarge,
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
    #[serde(default)]
    pub printer_name: Option<String>,
    #[serde(default)]
    pub file_url: Option<String>,
    #[serde(default)]
    pub data_base64: Option<String>,
    #[serde(default)]
    pub copies: Option<u16>,
    #[serde(default)]
    pub paper: Option<EffectivePaper>,
}

impl PrintJobInput {
    /// 校验任务字段组合是否可被接收进入队列。
    pub fn validate_for_acceptance(&self, max_file_size_mb: u64) -> Result<(), JobValidationError> {
        match self.format {
            SupportedFormat::Raw => self.validate_raw(max_file_size_mb),
            SupportedFormat::Pdf
            | SupportedFormat::Image
            | SupportedFormat::Png
            | SupportedFormat::Jpg
            | SupportedFormat::Jpeg
            | SupportedFormat::Docx
            | SupportedFormat::Xlsx
            | SupportedFormat::Pptx => self.validate_file_job(),
        }
    }

    fn validate_raw(&self, max_file_size_mb: u64) -> Result<(), JobValidationError> {
        if self.file_url.is_some() {
            return Err(JobValidationError::RawFileUrlNotAllowed);
        }
        if self.paper.is_some() {
            return Err(JobValidationError::RawPaperNotAllowed);
        }
        if self.copies.is_some() {
            return Err(JobValidationError::RawCopiesNotAllowed);
        }

        let data_base64 = self
            .data_base64
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or(JobValidationError::MissingRawData)?;
        let bytes = STANDARD
            .decode(data_base64)
            .map_err(|_| JobValidationError::InvalidRawData)?;
        let max_bytes = max_file_size_mb.saturating_mul(1024 * 1024);
        if bytes.len() as u64 > max_bytes {
            return Err(JobValidationError::FileTooLarge);
        }

        Ok(())
    }

    fn validate_file_job(&self) -> Result<(), JobValidationError> {
        if self
            .file_url
            .as_deref()
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(JobValidationError::MissingFileUrl);
        }
        if self.data_base64.is_some() {
            return Err(JobValidationError::FileRawDataNotAllowed);
        }

        Ok(())
    }
}

/// WebSocket 协议接受的浏览器客户端消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "ping")]
    Ping { time: i64 },
    #[serde(rename = "get_printers_list")]
    GetPrintersList { request_id: String },
    #[serde(rename = "get_printer_info")]
    GetPrinterInfo {
        request_id: String,
        printer_name: String,
    },
    #[serde(rename = "get_print_queue")]
    GetPrintQueue { request_id: String },
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
    #[serde(rename = "printers_list")]
    PrintersList {
        request_id: String,
        printers: Vec<PrinterInfo>,
    },
    #[serde(rename = "printer_info")]
    PrinterInfo {
        request_id: String,
        printer: PrinterDetails,
    },
    #[serde(rename = "print_queue")]
    PrintQueue {
        request_id: String,
        jobs: Vec<PrintQueueJobInfo>,
    },
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

/// 单台打印机及其可用纸张的 WebSocket 响应体。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterDetails {
    pub name: String,
    pub is_default: bool,
    pub dpi: Option<u32>,
    pub port: Option<String>,
    pub is_local: Option<bool>,
    pub is_network: Option<bool>,
    pub is_virtual: Option<bool>,
    pub papers: Vec<PaperInfo>,
    pub trays: Vec<PrinterTrayInfo>,
    pub media_types: Vec<PrinterMediaTypeInfo>,
}

/// 当前内存打印队列中的任务摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintQueueJobInfo {
    pub request_id: String,
    pub batch_id: Option<String>,
    pub job_id: String,
    pub status: JobStatus,
    pub message: Option<String>,
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
    OfficeConvertFailed,
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

#[cfg(test)]
mod tests {
    use super::{ErrorCode, JobValidationError, PrintJobInput, SupportedFormat};
    use std::str::FromStr;

    #[test]
    fn parses_office_supported_formats() {
        assert_eq!(
            SupportedFormat::from_str("docx").unwrap(),
            SupportedFormat::Docx
        );
        assert_eq!(
            SupportedFormat::from_str("xlsx").unwrap(),
            SupportedFormat::Xlsx
        );
        assert_eq!(
            SupportedFormat::from_str("pptx").unwrap(),
            SupportedFormat::Pptx
        );
    }

    #[test]
    fn serializes_office_convert_failed_error_code() {
        let value = serde_json::to_value(ErrorCode::OfficeConvertFailed).unwrap();
        assert_eq!(value, serde_json::json!("OFFICE_CONVERT_FAILED"));
    }

    #[test]
    fn raw_job_accepts_inline_base64_and_optional_printer_name() {
        let job: PrintJobInput = serde_json::from_str(
            r#"{
                "job_id":"RAW-001",
                "format":"raw",
                "printer_name":"Zebra ZD421",
                "data_base64":"XlhB"
            }"#,
        )
        .unwrap();

        assert_eq!(job.format, SupportedFormat::Raw);
        assert_eq!(job.printer_name.as_deref(), Some("Zebra ZD421"));
        assert_eq!(job.data_base64.as_deref(), Some("XlhB"));
        assert!(job.file_url.is_none());
        assert!(job.paper.is_none());
        assert!(job.copies.is_none());
        assert_eq!(job.validate_for_acceptance(20), Ok(()));
    }

    #[test]
    fn file_job_accepts_printer_name_and_existing_fields() {
        let job: PrintJobInput = serde_json::from_str(
            r#"{
                "job_id":"PDF-001",
                "format":"pdf",
                "printer_name":"Office Printer",
                "file_url":"https://example.com/a.pdf",
                "copies":2,
                "paper":{"width_mm":60,"height_mm":40}
            }"#,
        )
        .unwrap();

        assert_eq!(job.format, SupportedFormat::Pdf);
        assert_eq!(job.printer_name.as_deref(), Some("Office Printer"));
        assert_eq!(job.copies, Some(2));
        assert_eq!(job.validate_for_acceptance(20), Ok(()));
    }

    #[test]
    fn raw_job_rejects_file_fields() {
        let mut job = PrintJobInput {
            job_id: "RAW-INVALID".to_string(),
            format: SupportedFormat::Raw,
            printer_name: None,
            file_url: Some("https://example.com/raw.bin".to_string()),
            data_base64: Some("XlhB".to_string()),
            copies: None,
            paper: None,
        };

        assert_eq!(
            job.validate_for_acceptance(20),
            Err(JobValidationError::RawFileUrlNotAllowed)
        );

        job.file_url = None;
        job.copies = Some(1);
        assert_eq!(
            job.validate_for_acceptance(20),
            Err(JobValidationError::RawCopiesNotAllowed)
        );
    }
}
