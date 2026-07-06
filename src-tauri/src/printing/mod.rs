use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// 平台后端返回的打印机摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterInfo {
    pub name: String,
    pub is_default: bool,
}

/// 打印机支持或自定义兜底的纸张尺寸。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperInfo {
    pub id: String,
    pub name: String,
    pub width_mm: f64,
    pub height_mm: f64,
}

/// 后端提交单个 PDF 打印任务所需的选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintOptions {
    pub printer_name: String,
    pub paper: PaperInfo,
    pub copies: u16,
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

pub type PrintResult<T> = Result<T, PrintError>;

/// 队列 worker 和 HTTP API 使用的平台抽象。
pub trait PrintBackend {
    /// 列出已安装打印机。
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>>;
    /// 列出指定打印机的纸张。
    fn list_papers(&self, printer_name: &str) -> PrintResult<Vec<PaperInfo>>;
    /// 把 PDF 文件发送到平台打印队列。
    fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<PrintSubmission>;
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
}

/// 返回当前目标平台的打印后端。
pub fn default_backend() -> Box<dyn PrintBackend + Send + Sync> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacosPrintBackend)
    }

    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsPrintBackend::default())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Box::new(UnsupportedPrintBackend)
    }
}

/// 使用显式 SumatraPDF 可执行文件路径创建 Windows 后端。
#[cfg(target_os = "windows")]
pub fn windows_backend(
    sumatra_path: impl Into<std::path::PathBuf>,
) -> Box<dyn PrintBackend + Send + Sync> {
    Box::new(windows::WindowsPrintBackend::new(sumatra_path))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct UnsupportedPrintBackend;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl PrintBackend for UnsupportedPrintBackend {
    /// 报告不支持的平台，而不是返回假的打印机。
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
        Err(PrintError::UnsupportedPlatform)
    }

    /// 报告不支持的平台，而不是返回假的纸张。
    fn list_papers(&self, _printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
        Err(PrintError::UnsupportedPlatform)
    }

    /// 报告不支持的平台，而不是静默忽略打印任务。
    fn print_pdf(&self, _path: &Path, _options: &PrintOptions) -> PrintResult<PrintSubmission> {
        Err(PrintError::UnsupportedPlatform)
    }
}

/// 返回当前 UTC RFC3339 时间，用于记录平台提交时间。
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn submitted_at_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// 后端无法枚举纸张时使用的内置常见标签纸尺寸。
pub(crate) fn common_label_papers() -> Vec<PaperInfo> {
    vec![
        PaperInfo {
            id: "label_40x30".to_string(),
            name: "40 x 30 mm".to_string(),
            width_mm: 40.0,
            height_mm: 30.0,
        },
        PaperInfo {
            id: "label_50x30".to_string(),
            name: "50 x 30 mm".to_string(),
            width_mm: 50.0,
            height_mm: 30.0,
        },
        PaperInfo {
            id: "label_60x40".to_string(),
            name: "60 x 40 mm".to_string(),
            width_mm: 60.0,
            height_mm: 40.0,
        },
        PaperInfo {
            id: "label_80x50".to_string(),
            name: "80 x 50 mm".to_string(),
            width_mm: 80.0,
            height_mm: 50.0,
        },
        PaperInfo {
            id: "label_100x150".to_string(),
            name: "100 x 150 mm".to_string(),
            width_mm: 100.0,
            height_mm: 150.0,
        },
    ]
}

/// 返回 CUPS 介质 token；已有驱动 token 时保持原值。
pub(crate) fn cups_media_option(paper: &PaperInfo) -> String {
    if is_cups_media_token(&paper.id) {
        paper.id.clone()
    } else {
        custom_media_option(paper.width_mm, paper.height_mm)
    }
}

/// 格式化 SumatraPDF 打印设置，包括份数、适配模式和纸张尺寸。
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn sumatra_print_settings(copies: u16, paper: &PaperInfo) -> String {
    format!(
        "{}x,fit,paper={}mm x {}mm",
        copies.max(1),
        format_mm(paper.width_mm),
        format_mm(paper.height_mm)
    )
}

/// 根据纸张尺寸构造自定义 CUPS 介质选项。
pub(crate) fn custom_media_option(width_mm: f64, height_mm: f64) -> String {
    format!("Custom.{}x{}mm", format_mm(width_mm), format_mm(height_mm))
}

/// 根据毫米尺寸构造可读纸张名称。
pub(crate) fn paper_name(width_mm: f64, height_mm: f64) -> String {
    format!("{} x {} mm", format_mm(width_mm), format_mm(height_mm))
}

/// 格式化毫米值，去掉不必要的小数尾零。
fn format_mm(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

/// 查找尺寸与请求尺寸匹配的驱动纸张。
pub(crate) fn find_matching_paper<'a>(
    papers: &'a [PaperInfo],
    paper: &PaperInfo,
) -> Option<&'a PaperInfo> {
    papers.iter().find(|candidate| {
        (candidate.width_mm - paper.width_mm).abs() < 0.01
            && (candidate.height_mm - paper.height_mm).abs() < 0.01
    })
}

/// 有匹配的驱动纸张时使用它，否则保留自定义请求。
pub(crate) fn resolve_paper_for_print(papers: &[PaperInfo], paper: &PaperInfo) -> PaperInfo {
    find_matching_paper(papers, paper)
        .cloned()
        .unwrap_or_else(|| paper.clone())
}

/// 检测 CUPS 自定义介质 token。
fn is_cups_media_token(value: &str) -> bool {
    value.starts_with("Custom.")
}

#[cfg(test)]
mod tests {
    use super::{cups_media_option, resolve_paper_for_print, sumatra_print_settings, PaperInfo};

    #[test]
    fn sumatra_settings_include_copies_fit_and_explicit_paper_size() {
        let paper = PaperInfo {
            id: "label_60x40".to_string(),
            name: "60 x 40 mm".to_string(),
            width_mm: 60.0,
            height_mm: 40.0,
        };

        assert_eq!(
            sumatra_print_settings(2, &paper),
            "2x,fit,paper=60mm x 40mm"
        );
    }

    #[test]
    fn cups_media_prefers_driver_token_and_ignores_label_ids() {
        let driver_paper = PaperInfo {
            id: "Custom.62x38mm".to_string(),
            name: "62 x 38 mm".to_string(),
            width_mm: 62.0,
            height_mm: 38.0,
        };
        let builtin_paper = PaperInfo {
            id: "label_60x40".to_string(),
            name: "60 x 40 mm".to_string(),
            width_mm: 60.0,
            height_mm: 40.0,
        };

        assert_eq!(cups_media_option(&driver_paper), "Custom.62x38mm");
        assert_eq!(cups_media_option(&builtin_paper), "Custom.60x40mm");
    }

    #[test]
    fn resolve_paper_for_print_uses_custom_size_when_driver_has_no_match() {
        let driver_paper = PaperInfo {
            id: "Custom.62x38mm".to_string(),
            name: "62 x 38 mm".to_string(),
            width_mm: 62.0,
            height_mm: 38.0,
        };
        let requested = PaperInfo {
            id: "custom_37x19mm".to_string(),
            name: "37 x 19 mm".to_string(),
            width_mm: 37.0,
            height_mm: 19.0,
        };

        let resolved = resolve_paper_for_print(&[driver_paper], &requested);

        assert_eq!(resolved, requested);
        assert_eq!(cups_media_option(&resolved), "Custom.37x19mm");
        assert_eq!(
            sumatra_print_settings(1, &resolved),
            "1x,fit,paper=37mm x 19mm"
        );
    }
}
