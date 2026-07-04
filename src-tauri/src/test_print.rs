use crate::{
    app_state::AppState,
    config::AgentConfig,
    document::{calibration_page_to_pdf, DocumentError},
    printing::{paper_name, PaperInfo, PrintError, PrintOptions},
    protocol::EffectivePaper,
};
use std::path::PathBuf;
use thiserror::Error;

/// 测试打印校准页时可能返回的错误。
#[derive(Debug, Error)]
pub enum TestPrintError {
    #[error("printer not configured")]
    PrinterNotConfigured,
    #[error("paper not configured")]
    PaperNotConfigured,
    #[error("document generation failed: {0}")]
    Document(#[from] DocumentError),
    #[error("print failed: {0}")]
    Print(#[from] PrintError),
}

/// 使用当前默认打印设置生成并提交一张标签校准页。
pub async fn print_calibration_page(state: &AppState) -> Result<(), TestPrintError> {
    let config = state.config.read().await.clone();
    print_calibration_page_with_config(state, config).await
}

/// 使用指定配置生成并提交一张标签校准页，不持久化配置。
pub async fn print_calibration_page_with_config(
    state: &AppState,
    config: AgentConfig,
) -> Result<(), TestPrintError> {
    let printer_name = config
        .printing
        .default_printer
        .ok_or(TestPrintError::PrinterNotConfigured)?;
    let paper = config
        .printing
        .default_paper
        .ok_or(TestPrintError::PaperNotConfigured)?;
    let path = calibration_temp_path();
    let result = print_calibration_page_inner(state, printer_name, paper, &path).await;
    let _ = std::fs::remove_file(&path);

    result
}

async fn print_calibration_page_inner(
    state: &AppState,
    printer_name: String,
    paper: EffectivePaper,
    path: &PathBuf,
) -> Result<(), TestPrintError> {
    calibration_page_to_pdf(&paper, path)?;

    let options = PrintOptions {
        printer_name,
        paper: paper_info_from_effective(&paper),
        copies: 1,
    };
    let _print_guard = state.print_lock.lock().await;
    state.printing.print_pdf(path, &options)?;

    Ok(())
}

fn calibration_temp_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "print-bridge-calibration-{}.pdf",
        uuid::Uuid::new_v4()
    ))
}

fn paper_info_from_effective(paper: &EffectivePaper) -> PaperInfo {
    PaperInfo {
        id: format!(
            "custom_{}x{}mm",
            format_paper_dimension(paper.width_mm),
            format_paper_dimension(paper.height_mm)
        ),
        name: paper_name(paper.width_mm, paper.height_mm),
        width_mm: paper.width_mm,
        height_mm: paper.height_mm,
    }
}

fn format_paper_dimension(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}
