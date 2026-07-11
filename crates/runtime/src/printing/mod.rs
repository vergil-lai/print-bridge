pub use print_bridge_core::printing::{
    PaperInfo, PrintBackend, PrintError, PrintOptions, PrintResult, PrintSubmission,
    PrintTrackingOutcome, PrinterInfo, PrinterMediaTypeInfo, PrinterTrayInfo, RawPrintOptions,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod cups;
#[cfg(target_os = "windows")]
mod windows;

/// 返回当前目标平台的打印后端。
pub fn default_backend() -> Box<dyn PrintBackend + Send + Sync> {
    #[cfg(target_os = "macos")]
    {
        Box::new(cups::CupsPrintBackend::macos())
    }

    #[cfg(target_os = "linux")]
    {
        Box::new(cups::CupsPrintBackend::linux())
    }

    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsPrintBackend::default())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
struct UnsupportedPrintBackend;

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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
    fn print_pdf(
        &self,
        _path: &std::path::Path,
        _options: &PrintOptions,
    ) -> PrintResult<PrintSubmission> {
        Err(PrintError::UnsupportedPlatform)
    }

    /// 报告不支持的平台，而不是静默忽略 raw 打印任务。
    fn print_raw(&self, _data: &[u8], _options: &RawPrintOptions) -> PrintResult<PrintSubmission> {
        Err(PrintError::UnsupportedPlatform)
    }
}

/// 返回当前 UTC RFC3339 时间，用于记录平台提交时间。
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
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
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
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
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
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
