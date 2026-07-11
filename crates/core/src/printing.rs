use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// 平台后端返回的打印机摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterInfo {
    pub name: String,
    pub is_default: bool,
    pub dpi: Option<u32>,
    pub port: Option<String>,
    pub is_local: Option<bool>,
    pub is_network: Option<bool>,
    pub is_virtual: Option<bool>,
}

impl PrinterInfo {
    /// 构造只有名称和默认状态的摘要，未知平台字段保持为空。
    pub fn new(name: String, is_default: bool) -> Self {
        Self {
            name,
            is_default,
            dpi: None,
            port: None,
            is_local: None,
            is_network: None,
            is_virtual: None,
        }
    }
}

/// 打印机支持或自定义兜底的纸张尺寸。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperInfo {
    pub id: String,
    pub name: String,
    pub width_mm: f64,
    pub height_mm: f64,
}

/// 打印机纸盒或进纸来源选项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterTrayInfo {
    pub id: String,
    pub name: String,
}

/// 打印机介质类型选项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterMediaTypeInfo {
    pub id: String,
    pub name: String,
}

/// 后端提交单个 PDF 打印任务所需的选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintOptions {
    pub printer_name: String,
    pub paper: PaperInfo,
    pub copies: u16,
}

/// 后端提交单个 raw 打印任务所需的选项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPrintOptions {
    pub printer_name: String,
}

/// 平台打印命令成功提交后的可追踪信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintSubmission {
    pub submitted_at: String,
    pub backend: String,
    pub system_job_id: Option<String>,
    pub tracking_supported: bool,
}

/// 平台队列追踪的保守结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrintTrackingOutcome {
    Completed { message: String },
    Failed { message: String },
    Unknown { message: String },
}

/// 平台打印后端返回的错误。
#[derive(Debug, Error)]
pub enum PrintError {
    #[error("printer not found: {0}")]
    PrinterNotFound(String),
    #[error("paper not found: {0}")]
    PaperNotFound(String),
    #[error("print command failed: {command}: {message}")]
    CommandFailed { command: String, message: String },
    #[error("printing is not supported on this platform")]
    UnsupportedPlatform,
}

/// 平台打印后端统一返回结果。
pub type PrintResult<T> = Result<T, PrintError>;

/// 队列 worker 和功能入口使用的平台打印抽象。
pub trait PrintBackend {
    /// 列出已安装打印机。
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>>;
    /// 列出指定打印机的纸张。
    fn list_papers(&self, printer_name: &str) -> PrintResult<Vec<PaperInfo>>;
    /// 列出指定打印机可用纸盒；平台不支持枚举时返回空列表。
    fn list_trays(&self, _printer_name: &str) -> PrintResult<Vec<PrinterTrayInfo>> {
        Ok(Vec::new())
    }
    /// 列出指定打印机可用介质类型；平台不支持枚举时返回空列表。
    fn list_media_types(&self, _printer_name: &str) -> PrintResult<Vec<PrinterMediaTypeInfo>> {
        Ok(Vec::new())
    }
    /// 把 PDF 文件发送到平台打印队列。
    fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<PrintSubmission>;
    /// 把原始打印指令字节发送到平台打印队列。
    fn print_raw(&self, data: &[u8], options: &RawPrintOptions) -> PrintResult<PrintSubmission>;
    /// 查询平台队列对本次提交的保守状态。
    fn track_submission(
        &self,
        _submission: &PrintSubmission,
        _options: &PrintOptions,
    ) -> PrintTrackingOutcome {
        PrintTrackingOutcome::Unknown {
            message: "platform does not provide trackable print status".to_string(),
        }
    }
    /// 查询平台队列对 raw 提交的保守状态。
    fn track_raw_submission(
        &self,
        _submission: &PrintSubmission,
        _options: &RawPrintOptions,
    ) -> PrintTrackingOutcome {
        PrintTrackingOutcome::Unknown {
            message: "platform does not provide trackable raw print status".to_string(),
        }
    }
}
