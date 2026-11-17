pub use print_bridge_core::printing::{
    PaperInfo, PrintBackend, PrintError, PrintOptions, PrintResult, PrintSubmission,
    PrintTrackingOutcome, PrinterInfo, PrinterMediaTypeInfo, PrinterTrayInfo, RawPrintOptions,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod cups;
pub mod diagnostics;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(any(test, target_os = "windows"))]
mod windows_buffer;

/// 将像素测量值对应到表单的有序尺寸，保留纸张方向并容忍设备像素量化。
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn windows_physical_paper_dimensions(
    form_width: f64,
    form_height: f64,
    physical_width: f64,
    physical_height: f64,
) -> Option<(f64, f64)> {
    let error = |width: f64, height: f64| {
        (width - physical_width)
            .abs()
            .max((height - physical_height).abs())
    };
    let direct = error(form_width, form_height);
    let rotated = error(form_height, form_width);
    if direct < 1.0 && direct <= rotated {
        Some((form_width, form_height))
    } else if rotated < 1.0 {
        Some((form_height, form_width))
    } else {
        None
    }
}

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
#[cfg(any(target_os = "macos", target_os = "linux"))]
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

/// 按任务尺寸匹配 Windows 驱动纸型，并构造 SumatraPDF 的份数、方向和选纸参数。
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn sumatra_print_settings(
    copies: u16,
    requested: &PaperInfo,
    driver_papers: &[PaperInfo],
) -> PrintResult<String> {
    let matches = |width: f64, height: f64| {
        (width - requested.width_mm).abs() < 0.1 && (height - requested.height_mm).abs() < 0.1
    };
    let paper = driver_papers
        .iter()
        .find(|paper| matches(paper.width_mm, paper.height_mm))
        .ok_or_else(|| {
            PrintError::PaperNotFound(format!(
                "{}; configure this paper size and orientation in the Windows printer driver first",
                paper_name(requested.width_mm, requested.height_mm)
            ))
        })?;
    let paper_kind = paper
        .id
        .parse::<u16>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| PrintError::PaperNotFound(paper.name.clone()))?;
    let orientation = if requested.width_mm > requested.height_mm {
        "landscape"
    } else {
        "portrait"
    };

    // paperkind 使用驱动编号，不依赖本地化纸型名称，也不让 PDF 首页覆盖任务选纸。
    Ok(format!(
        "{}x,fit,{},paperkind={}",
        copies.max(1),
        orientation,
        paper_kind
    ))
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
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
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
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
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
    fn windows_paper_uses_measured_orientation_and_nominal_dimensions() {
        assert_eq!(
            super::windows_physical_paper_dimensions(210.0, 297.0, 296.9, 209.9),
            Some((297.0, 210.0))
        );
        assert_eq!(
            super::windows_physical_paper_dimensions(230.0, 140.0, 229.9, 140.1),
            Some((230.0, 140.0))
        );
        assert_eq!(
            super::windows_physical_paper_dimensions(230.0, 140.0, 210.0, 297.0),
            None
        );
        assert_eq!(
            super::windows_physical_paper_dimensions(230.0, 230.5, 230.5, 230.0),
            Some((230.5, 230.0))
        );
    }

    #[test]
    fn sumatra_settings_select_custom_driver_paper_and_landscape() {
        let requested = windows_paper("custom_230x140mm", 230.0, 140.0);
        let driver = windows_paper("256", 230.0, 140.0);
        assert_eq!(
            sumatra_print_settings(2, &requested, &[driver]).unwrap(),
            "2x,fit,landscape,paperkind=256"
        );
    }

    #[test]
    fn sumatra_settings_require_at_least_one_copy() {
        let paper = windows_paper("9", 210.0, 297.0);
        assert_eq!(
            sumatra_print_settings(0, &paper, std::slice::from_ref(&paper)).unwrap(),
            "1x,fit,portrait,paperkind=9"
        );
    }

    #[test]
    fn sumatra_settings_reject_rotated_physical_paper() {
        let requested = windows_paper("custom_297x210mm", 297.0, 210.0);
        let driver = windows_paper("9", 210.0, 297.0);
        assert!(sumatra_print_settings(1, &requested, &[driver]).is_err());
    }

    #[test]
    fn sumatra_settings_accept_landscape_only_when_driver_physical_page_is_landscape() {
        let requested = windows_paper("custom_297x210mm", 297.0, 210.0);
        for ((physical_width, physical_height), allowed) in
            [((210.0, 297.0), false), ((297.0, 210.0), true)]
        {
            let (width, height) = super::windows_physical_paper_dimensions(
                210.0,
                297.0,
                physical_width,
                physical_height,
            )
            .unwrap();
            let driver = windows_paper("9", width, height);
            let settings = sumatra_print_settings(1, &requested, &[driver]);
            assert_eq!(settings.is_ok(), allowed);
            if allowed {
                assert_eq!(settings.unwrap(), "1x,fit,landscape,paperkind=9");
            }
        }
    }

    #[test]
    fn sumatra_settings_prefer_exact_dimensions_over_rotated_match() {
        let requested = windows_paper("custom", 230.0, 140.0);
        let rotated = windows_paper("256", 140.0, 230.0);
        let exact = windows_paper("257", 230.0, 140.0);
        assert!(sumatra_print_settings(1, &requested, &[rotated, exact])
            .unwrap()
            .ends_with("paperkind=257"));
    }

    #[test]
    fn sumatra_settings_reject_missing_driver_paper_instead_of_using_pdf_size() {
        let requested = windows_paper("custom", 230.0, 140.0);
        let a4 = windows_paper("9", 210.0, 297.0);
        assert!(matches!(
            sumatra_print_settings(1, &requested, &[a4]),
            Err(super::PrintError::PaperNotFound(_))
        ));
    }

    #[test]
    fn sumatra_settings_reject_synthetic_paper_ids() {
        let paper = windows_paper("label_60x40", 60.0, 40.0);
        assert!(sumatra_print_settings(1, &paper, std::slice::from_ref(&paper)).is_err());
    }

    #[test]
    fn sumatra_settings_keep_portrait_for_tall_and_square_paper() {
        for (width, height) in [(140.0, 230.0), (50.0, 50.0)] {
            let paper = windows_paper("256", width, height);
            assert_eq!(
                sumatra_print_settings(1, &paper, std::slice::from_ref(&paper)).unwrap(),
                "1x,fit,portrait,paperkind=256"
            );
        }
    }

    fn windows_paper(id: &str, width_mm: f64, height_mm: f64) -> PaperInfo {
        PaperInfo {
            id: id.to_string(),
            name: "2等分2".to_string(),
            width_mm,
            height_mm,
        }
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
    }
}
