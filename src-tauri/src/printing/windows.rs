use super::{
    common_label_papers, resolve_paper_for_print, sumatra_print_settings, PaperInfo, PrintBackend,
    PrintError, PrintOptions, PrintResult, PrinterInfo,
};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Windows 打印后端：用 PowerShell 发现打印机，用 SumatraPDF 执行打印。
#[derive(Debug, Clone)]
pub struct WindowsPrintBackend {
    sumatra_path: PathBuf,
}

impl Default for WindowsPrintBackend {
    /// 使用应用二进制文件旁边打包的 SumatraPDF 可执行文件。
    fn default() -> Self {
        Self {
            sumatra_path: bundled_sumatra_path(),
        }
    }
}

impl WindowsPrintBackend {
    /// 使用显式 SumatraPDF 路径创建后端，主要用于应用初始化和测试。
    pub fn new(sumatra_path: impl Into<PathBuf>) -> Self {
        Self {
            sumatra_path: sumatra_path.into(),
        }
    }
}

impl PrintBackend for WindowsPrintBackend {
    /// 通过 PowerShell JSON 输出列出 Windows 打印机。
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-Printer | Select-Object Name,Default | ConvertTo-Json",
            ])
            .output()
            .map_err(|error| command_error("powershell", error.to_string()))?;

        if !output.status.success() {
            return Err(command_error(
                "powershell",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        parse_printers_json(&String::from_utf8_lossy(&output.stdout))
    }

    /// 确认打印机存在后返回常见标签纸尺寸。
    fn list_papers(&self, printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
        ensure_printer_exists(self, printer_name)?;
        Ok(common_label_papers())
    }

    /// 使用明确的打印机和纸张设置把 PDF 发送给 SumatraPDF。
    fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<()> {
        ensure_printer_exists(self, &options.printer_name)?;
        let paper = resolve_print_paper(self, options)?;

        let settings = sumatra_print_settings(options.copies, &paper);
        let output = Command::new(&self.sumatra_path)
            .arg("-silent")
            .arg("-print-to")
            .arg(&options.printer_name)
            .arg("-print-settings")
            .arg(settings)
            .arg(path)
            .output()
            .map_err(|error| command_error("SumatraPDF.exe", error.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(command_error(
                "SumatraPDF.exe",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ))
        }
    }
}

/// PowerShell 的 ConvertTo-Json 可能返回单个对象或数组。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PrinterJson {
    One(PowerShellPrinter),
    Many(Vec<PowerShellPrinter>),
}

/// PowerShell 返回的原始打印机结构。
#[derive(Debug, Deserialize)]
struct PowerShellPrinter {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Default", default)]
    default: bool,
}

/// 把 PowerShell 打印机 JSON 解析为平台无关的打印机摘要。
fn parse_printers_json(value: &str) -> PrintResult<Vec<PrinterInfo>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }

    let printers = serde_json::from_str::<PrinterJson>(value).map_err(|error| {
        command_error(
            "powershell",
            format!("failed to parse printer json: {error}"),
        )
    })?;

    Ok(match printers {
        PrinterJson::One(printer) => vec![printer.into()],
        PrinterJson::Many(printers) => printers.into_iter().map(Into::into).collect(),
    })
}

impl From<PowerShellPrinter> for PrinterInfo {
    /// 把 PowerShell 字段名转换为共享打印机类型。
    fn from(value: PowerShellPrinter) -> Self {
        Self {
            name: value.name,
            is_default: value.default,
        }
    }
}

/// Ensures a printer exists before paper lookup or printing.
fn ensure_printer_exists(backend: &WindowsPrintBackend, printer_name: &str) -> PrintResult<()> {
    if backend
        .list_printers()?
        .iter()
        .any(|printer| printer.name == printer_name)
    {
        Ok(())
    } else {
        Err(PrintError::PrinterNotFound(printer_name.to_string()))
    }
}

/// 根据后端可用纸张列表解析请求纸张。
fn resolve_print_paper(
    backend: &WindowsPrintBackend,
    options: &PrintOptions,
) -> PrintResult<PaperInfo> {
    let papers = backend.list_papers(&options.printer_name)?;
    Ok(resolve_paper_for_print(&papers, &options.paper))
}

/// 在当前可执行文件旁查找打包的 SumatraPDF，并提供 PATH 兜底。
fn bundled_sumatra_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("SumatraPDF.exe")))
        .unwrap_or_else(|| PathBuf::from("SumatraPDF.exe"))
}

/// 为失败的平台命令构造 PrintError。
fn command_error(command: &str, message: String) -> PrintError {
    PrintError::CommandFailed {
        command: command.to_string(),
        message,
    }
}
