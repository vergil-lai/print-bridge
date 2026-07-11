use super::{
    common_label_papers, resolve_paper_for_print, submitted_at_rfc3339, sumatra_print_settings,
    PaperInfo, PrintBackend, PrintError, PrintOptions, PrintResult, PrintSubmission,
    PrintTrackingOutcome, PrinterInfo, RawPrintOptions,
};
use serde::Deserialize;
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
    ptr,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, EndDocPrinter, EndPagePrinter, OpenPrinterW, StartDocPrinterW, StartPagePrinter,
    WritePrinter, DOC_INFO_1W, PRINTER_HANDLE,
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
                "Get-Printer | ForEach-Object { $config = Get-PrintConfiguration -PrinterName $_.Name -ErrorAction SilentlyContinue; [PSCustomObject]@{ Name=$_.Name; Default=$_.Default; PortName=$_.PortName; Type=$_.Type.ToString(); DeviceType=$_.DeviceType.ToString(); DriverName=$_.DriverName; PrintQuality=if ($config) { $config.PrintQuality } else { $null } } } | ConvertTo-Json",
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
    fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<PrintSubmission> {
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
            Ok(PrintSubmission {
                submitted_at: submitted_at_rfc3339(),
                backend: "windows-sumatra".to_string(),
                system_job_id: None,
                tracking_supported: false,
            })
        } else {
            Err(command_error(
                "SumatraPDF.exe",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ))
        }
    }

    /// 使用 Windows RAW spooler 把原始打印指令提交给打印机。
    fn print_raw(&self, data: &[u8], options: &RawPrintOptions) -> PrintResult<PrintSubmission> {
        ensure_printer_exists(self, &options.printer_name)?;
        submit_raw_to_windows_printer(&options.printer_name, data)?;

        Ok(PrintSubmission {
            submitted_at: submitted_at_rfc3339(),
            backend: "windows-raw-spooler".to_string(),
            system_job_id: None,
            tracking_supported: false,
        })
    }

    fn track_submission(
        &self,
        _submission: &PrintSubmission,
        _options: &PrintOptions,
    ) -> PrintTrackingOutcome {
        PrintTrackingOutcome::Unknown {
            message: "Windows SumatraPDF submission did not expose a system print job id"
                .to_string(),
        }
    }
}

fn submit_raw_to_windows_printer(printer_name: &str, data: &[u8]) -> PrintResult<()> {
    let printer_name_w = wide_null(printer_name);
    let document_name_w = wide_null("PrintBridge Raw Job");
    let data_type_w = wide_null("RAW");
    let mut printer = PRINTER_HANDLE {
        Value: ptr::null_mut(),
    };

    unsafe {
        if OpenPrinterW(
            printer_name_w.as_ptr() as *mut _,
            &mut printer,
            ptr::null_mut(),
        ) == 0
        {
            return Err(command_error("OpenPrinterW", last_os_error()));
        }

        let doc_info = DOC_INFO_1W {
            pDocName: document_name_w.as_ptr() as *mut _,
            pOutputFile: ptr::null_mut(),
            pDatatype: data_type_w.as_ptr() as *mut _,
        };

        if StartDocPrinterW(printer, 1, &doc_info as *const _ as *mut _) == 0 {
            ClosePrinter(printer);
            return Err(command_error("StartDocPrinterW", last_os_error()));
        }

        if StartPagePrinter(printer) == 0 {
            EndDocPrinter(printer);
            ClosePrinter(printer);
            return Err(command_error("StartPagePrinter", last_os_error()));
        }

        let mut written = 0_u32;
        let ok = WritePrinter(
            printer,
            data.as_ptr() as *const _,
            data.len() as u32,
            &mut written,
        );

        EndPagePrinter(printer);
        EndDocPrinter(printer);
        ClosePrinter(printer);

        if ok == 0 || written as usize != data.len() {
            return Err(command_error("WritePrinter", last_os_error()));
        }
    }

    Ok(())
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn last_os_error() -> String {
    std::io::Error::last_os_error().to_string()
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
    #[serde(rename = "PortName", default)]
    port_name: Option<String>,
    #[serde(rename = "Type", default)]
    printer_type: Option<String>,
    #[serde(rename = "DeviceType", default)]
    device_type: Option<String>,
    #[serde(rename = "DriverName", default)]
    driver_name: Option<String>,
    #[serde(rename = "PrintQuality", default)]
    print_quality: Option<serde_json::Value>,
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
        let dpi = value
            .print_quality
            .as_ref()
            .and_then(parse_print_quality_dpi);
        let port = value.port_name.clone();
        let (is_local, is_network, is_virtual) = classify_windows_printer(
            value.name.as_str(),
            value.printer_type.as_deref(),
            value.device_type.as_deref(),
            value.driver_name.as_deref(),
            port.as_deref(),
        );

        Self {
            name: value.name,
            is_default: value.default,
            dpi,
            port,
            is_local,
            is_network,
            is_virtual,
        }
    }
}

fn parse_print_quality_dpi(value: &serde_json::Value) -> Option<u32> {
    match value {
        serde_json::Value::Number(number) => {
            number.as_u64().and_then(|value| value.try_into().ok())
        }
        serde_json::Value::String(value) => parse_dpi_text(value),
        _ => None,
    }
}

fn parse_dpi_text(value: &str) -> Option<u32> {
    let lower = value.to_ascii_lowercase();
    let dpi_index = lower.find("dpi").unwrap_or(lower.len());
    lower[..dpi_index]
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse::<u32>().ok())
        .max()
}

fn classify_windows_printer(
    name: &str,
    printer_type: Option<&str>,
    device_type: Option<&str>,
    driver_name: Option<&str>,
    port: Option<&str>,
) -> (Option<bool>, Option<bool>, Option<bool>) {
    let text = [Some(name), printer_type, device_type, driver_name, port]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let port = port.unwrap_or_default().to_ascii_lowercase();

    let is_virtual = text.contains("pdf")
        || text.contains("xps")
        || text.contains("onenote")
        || text.contains("fax")
        || text.contains("document writer");
    let is_network = port.starts_with("\\\\")
        || port.starts_with("ip_")
        || port.starts_with("tcp")
        || port.starts_with("http")
        || port.starts_with("wsd")
        || text.contains("connection")
        || text.contains("network");
    let is_local = port.starts_with("usb")
        || port.starts_with("lpt")
        || port.starts_with("com")
        || text.contains("local");

    if is_virtual {
        (Some(false), Some(false), Some(true))
    } else if is_network {
        (Some(false), Some(true), Some(false))
    } else if is_local {
        (Some(true), Some(false), Some(false))
    } else {
        (None, None, None)
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
