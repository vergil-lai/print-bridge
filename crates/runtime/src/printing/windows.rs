use super::diagnostics::record;
use super::{
    submitted_at_rfc3339, sumatra_print_settings,
    windows_buffer::{read_buffer, BufferRead},
    windows_physical_paper_dimensions, PaperInfo, PrintBackend, PrintError, PrintOptions,
    PrintResult, PrintSubmission, PrintTrackingOutcome, PrinterInfo, RawPrintOptions,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    ffi::OsStr,
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    ptr,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EndDocPrinter, EndPagePrinter, EnumFormsW, GetPrinterW,
    OpenPrinterW, StartDocPrinterW, StartPagePrinter, WritePrinter, DOC_INFO_1W, FORM_INFO_1W,
    PRINTER_HANDLE, PRINTER_INFO_8W, PRINTER_INFO_9W,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_INSUFFICIENT_BUFFER},
    Graphics::Gdi::{
        CreateICW, DeleteDC, GetDeviceCaps, DEVMODEW, DM_FORMNAME, DM_IN_BUFFER, DM_OUT_BUFFER,
        DM_PAPERLENGTH, DM_PAPERSIZE, DM_PAPERWIDTH, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT,
        PHYSICALWIDTH,
    },
};

const CREATE_NO_WINDOW: u32 = 0x08000000;

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
        let output = hidden_command("powershell")
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

    /// 读取可用纸型编号及当前驱动方向下已验证的物理毫米尺寸。
    fn list_papers(&self, printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
        ensure_printer_exists(self, printer_name)?;
        read_driver_papers(printer_name)
    }

    /// 按任务尺寸选择驱动纸型，使用明确的份数和内容方向发送 PDF。
    fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<PrintSubmission> {
        let operation = uuid::Uuid::new_v4().to_string();
        record(
            &operation,
            &options.printer_name,
            "print_request",
            json!({
                "width_mm": options.paper.width_mm, "height_mm": options.paper.height_mm,
                "copies": options.copies,
            }),
        );
        let papers = self
            .list_papers(&options.printer_name)
            .inspect_err(|error| {
                record(
                    &operation,
                    &options.printer_name,
                    "paper_lookup_failed",
                    json!({"error": error.to_string()}),
                );
            })?;
        let settings = sumatra_print_settings(options.copies, &options.paper, &papers)
            .inspect_err(|error| {
                record(
                    &operation,
                    &options.printer_name,
                    "paper_match_failed",
                    json!({
                        "error": error.to_string(), "available_papers": papers,
                        "width_mm": options.paper.width_mm, "height_mm": options.paper.height_mm,
                        "tolerance_mm": 0.1,
                    }),
                );
            })?;
        record(
            &operation,
            &options.printer_name,
            "paper_matched",
            json!({"settings": settings}),
        );
        let output = hidden_command(&self.sumatra_path)
            .arg("-silent")
            .arg("-print-to")
            .arg(&options.printer_name)
            .arg("-print-settings")
            .arg(&settings)
            .arg(path)
            .output()
            .inspect_err(|error| {
                record(
                    &operation,
                    &options.printer_name,
                    "submission_failed",
                    json!({"error": error.to_string()}),
                );
            })
            .map_err(|error| command_error("SumatraPDF.exe", error.to_string()))?;
        record(
            &operation,
            &options.printer_name,
            "submission_result",
            json!({"success": output.status.success(), "exit_code": output.status.code()}),
        );

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

/// 安全枚举表单，再验证驱动编号在 SumatraPDF 等效配置下的物理尺寸。
fn read_driver_papers(printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
    let operation = uuid::Uuid::new_v4().to_string();
    record(&operation, printer_name, "enumeration_started", json!({}));
    let name = wide_null(printer_name);
    let mut printer = PRINTER_HANDLE {
        Value: ptr::null_mut(),
    };
    if unsafe { OpenPrinterW(name.as_ptr(), &mut printer, ptr::null()) } == 0 {
        let error = command_error("OpenPrinterW", last_os_error());
        record(
            &operation,
            printer_name,
            "enumeration_failed",
            json!({"error": error.to_string()}),
        );
        return Err(error);
    }
    let result = read_validated_papers(printer, &name, &operation, printer_name);
    unsafe { ClosePrinter(printer) };
    match &result {
        Ok(papers) => record(
            &operation,
            printer_name,
            "enumeration_completed",
            json!({"papers": papers, "count": papers.len()}),
        ),
        Err(error) => record(
            &operation,
            printer_name,
            "enumeration_failed",
            json!({"error": error.to_string()}),
        ),
    }
    result
}

/// 对已打开的打印机读取表单，保留用户默认设置用于纸型编号的等效验证。
fn read_validated_papers(
    printer: PRINTER_HANDLE,
    name: &[u16],
    operation: &str,
    printer_name: &str,
) -> PrintResult<Vec<PaperInfo>> {
    let trace = |stage: &str, details| record(operation, printer_name, stage, details);
    let default_mode = read_default_devmode(printer, name)?;
    trace("default_devmode", devmode_summary(&default_mode));
    let (forms, count) = read_buffer("EnumFormsW", |buffer| {
        let mut needed = 0;
        let mut count = 0;
        let output = if buffer.is_empty() {
            ptr::null_mut()
        } else {
            buffer.as_mut_ptr().cast()
        };
        let ok = unsafe {
            EnumFormsW(
                printer,
                1,
                output,
                (buffer.len() * 8) as u32,
                &mut needed,
                &mut count,
            )
        };
        buffer_read_result(ok, needed, count)
    })?;
    if count > forms.len() * 8 / std::mem::size_of::<FORM_INFO_1W>() {
        return Err(command_error("EnumFormsW", "invalid form count".into()));
    }
    trace("forms_enumerated", json!({"count": count}));
    let mut papers = Vec::new();
    for index in 0..count {
        let form = unsafe { &*forms.as_ptr().cast::<FORM_INFO_1W>().add(index) };
        trace(
            "form_read",
            json!({"index": index, "width_mm": f64::from(form.Size.cx) / 1000.0, "height_mm": f64::from(form.Size.cy) / 1000.0}),
        );
        if form.Size.cx <= 0 || form.Size.cy <= 0 || form.pName.is_null() {
            trace(
                "form_rejected",
                json!({"index": index, "reason": "invalid_dimensions_or_missing_name"}),
            );
            continue;
        }
        let start = form.pName as usize;
        let base = forms.as_ptr() as usize;
        let end = base + forms.len() * 8;
        if start < base || start >= end || !start.is_multiple_of(2) {
            return Err(command_error("EnumFormsW", "invalid form name".into()));
        }
        let raw_name =
            unsafe { std::slice::from_raw_parts(form.pName, ((end - start) / 2).min(32)) };
        let Some(length) = raw_name.iter().position(|value| *value == 0) else {
            trace(
                "form_rejected",
                json!({"index": index, "reason": "name_exceeds_devmode_capacity", "name_prefix": String::from_utf16_lossy(raw_name)}),
            );
            continue;
        };
        trace(
            "form_name",
            json!({"index": index, "name": String::from_utf16_lossy(&raw_name[..length])}),
        );
        let mut candidate = default_mode.clone();
        let mode = unsafe { &mut *candidate.as_mut_ptr().cast::<DEVMODEW>() };
        mode.dmFormName.fill(0);
        mode.dmFormName[..length].copy_from_slice(&raw_name[..length]);
        mode.dmFields &= !(DM_PAPERSIZE | DM_PAPERWIDTH | DM_PAPERLENGTH);
        mode.dmFields |= DM_FORMNAME;
        let status = validate_devmode(printer, name, &mut candidate);
        if status != 1 {
            let error = last_os_error();
            trace(
                "form_rejected",
                json!({"index": index, "reason": "form_name_validation_failed", "return_code": status, "last_os_error": error}),
            );
            continue;
        }
        trace(
            "form_name_resolved",
            json!({"index": index, "devmode": devmode_summary(&candidate)}),
        );
        let kind = unsafe {
            (*candidate.as_ptr().cast::<DEVMODEW>())
                .Anonymous1
                .Anonymous1
                .dmPaperSize
        };
        if kind <= 0
            || papers
                .iter()
                .any(|paper: &PaperInfo| paper.id == kind.to_string())
        {
            trace(
                "form_rejected",
                json!({"index": index, "kind": kind, "reason": if kind <= 0 {"invalid_paper_kind"} else {"duplicate_paper_kind"}}),
            );
            continue;
        }
        // SumatraPDF 3.6.1 只覆盖默认 DEVMODE 的纸型编号，再交给驱动验证。
        let mut effective = default_mode.clone();
        let mode = unsafe { &mut *effective.as_mut_ptr().cast::<DEVMODEW>() };
        mode.Anonymous1.Anonymous1.dmPaperSize = kind;
        mode.dmFields |= DM_PAPERSIZE;
        let status = validate_devmode(printer, name, &mut effective);
        if status != 1 {
            let error = last_os_error();
            trace(
                "form_rejected",
                json!({"index": index, "kind": kind, "reason": "paper_kind_validation_failed", "return_code": status, "last_os_error": error}),
            );
            continue;
        }
        trace(
            "paper_kind_resolved",
            json!({"index": index, "requested_kind": kind, "devmode": devmode_summary(&effective)}),
        );
        let (width_mm, height_mm) = match physical_paper_size(name, &effective) {
            Ok(size) => size,
            Err(error) => {
                trace(
                    "form_rejected",
                    json!({"index": index, "kind": kind, "reason": "physical_size_unavailable", "error": error.to_string()}),
                );
                continue;
            }
        };
        trace(
            "physical_size",
            json!({"index": index, "kind": kind, "width_mm": width_mm, "height_mm": height_mm}),
        );
        let form_width = f64::from(form.Size.cx) / 1000.0;
        let form_height = f64::from(form.Size.cy) / 1000.0;
        let Some((width_mm, height_mm)) =
            windows_physical_paper_dimensions(form_width, form_height, width_mm, height_mm)
        else {
            trace(
                "form_rejected",
                json!({"index": index, "kind": kind, "reason": "physical_size_mismatch", "form_width_mm": form_width, "form_height_mm": form_height, "physical_width_mm": width_mm, "physical_height_mm": height_mm}),
            );
            continue;
        };
        papers.push(PaperInfo {
            id: kind.to_string(),
            name: String::from_utf16_lossy(&raw_name[..length]),
            width_mm,
            height_mm,
        });
        trace(
            "form_accepted",
            json!({"index": index, "paper": papers.last()}),
        );
    }
    Ok(papers)
}

/// 仅提取公开选纸字段，不导出驱动私有字节或指针。
fn devmode_summary(mode: &[u64]) -> serde_json::Value {
    let mode = unsafe { &*mode.as_ptr().cast::<DEVMODEW>() };
    let paper = unsafe { mode.Anonymous1.Anonymous1 };
    let length = mode
        .dmFormName
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(mode.dmFormName.len());
    json!({"fields": mode.dmFields, "paper_kind": paper.dmPaperSize,
        "paper_width_tenth_mm": paper.dmPaperWidth, "paper_length_tenth_mm": paper.dmPaperLength,
        "orientation": paper.dmOrientation, "form_name": String::from_utf16_lossy(&mode.dmFormName[..length]),
        "driver_version": mode.dmDriverVersion, "driver_extra_bytes": mode.dmDriverExtra})
}

/// 读取 Windows 版本、打印机驱动关联及驱动版本，不包含用户身份或凭据。
pub(super) fn diagnostic_environment() -> PrintResult<serde_json::Value> {
    let output = hidden_command("powershell").args([
        "-NoProfile", "-Command",
        "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [PSCustomObject]@{ OS=(Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber); Printers=@(Get-Printer | Select-Object Name,DriverName); Drivers=@(Get-PrinterDriver | ForEach-Object { [PSCustomObject]@{ Name=$_.Name; Manufacturer=$_.Manufacturer; MajorVersion=$_.MajorVersion; FileVersion=if ($_.DriverPath -and (Test-Path -LiteralPath $_.DriverPath)) { [System.Diagnostics.FileVersionInfo]::GetVersionInfo($_.DriverPath).FileVersion } else { $null } } }) } | ConvertTo-Json -Depth 4",
    ]).output().map_err(|error| command_error("powershell diagnostics", error.to_string()))?;
    if !output.status.success() {
        return Err(command_error(
            "powershell diagnostics",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| command_error("powershell diagnostics JSON", error.to_string()))
}

/// 把带容量 Win32 查询的返回值转换为重试或失败结果。
fn buffer_read_result(ok: i32, needed: u32, count: u32) -> BufferRead {
    if ok != 0 {
        BufferRead::Complete(count as usize)
    } else if unsafe { GetLastError() } == ERROR_INSUFFICIENT_BUFFER {
        BufferRead::Resize(needed as usize)
    } else {
        BufferRead::Failed(last_os_error())
    }
}

/// 读取用户默认 DEVMODE，未设置用户配置时使用全局默认配置。
fn read_default_devmode(printer: PRINTER_HANDLE, name: &[u16]) -> PrintResult<Vec<u64>> {
    for level in [9, 8] {
        let (buffer, _) = read_buffer("GetPrinterW", |buffer| {
            let mut needed = 0;
            let output = if buffer.is_empty() {
                ptr::null_mut()
            } else {
                buffer.as_mut_ptr().cast()
            };
            let ok = unsafe {
                GetPrinterW(
                    printer,
                    level,
                    output,
                    (buffer.len() * 8) as u32,
                    &mut needed,
                )
            };
            buffer_read_result(ok, needed, 1)
        })?;
        if buffer.len() * 8 < std::mem::size_of::<PRINTER_INFO_9W>() {
            return Err(command_error(
                "GetPrinterW",
                "invalid printer configuration".into(),
            ));
        }
        let source = unsafe {
            if level == 9 {
                (*buffer.as_ptr().cast::<PRINTER_INFO_9W>()).pDevMode
            } else {
                (*buffer.as_ptr().cast::<PRINTER_INFO_8W>()).pDevMode
            }
        };
        if source.is_null() {
            continue;
        }
        let start = source as usize;
        let base = buffer.as_ptr() as usize;
        let end = base + buffer.len() * 8;
        if start < base || start + std::mem::size_of::<DEVMODEW>() > end {
            return Err(command_error(
                "GetPrinterW",
                "invalid DEVMODE buffer".into(),
            ));
        }
        let public_size = unsafe { usize::from((*source).dmSize) };
        let length = public_size + unsafe { usize::from((*source).dmDriverExtra) };
        if public_size < std::mem::size_of::<DEVMODEW>() || start + length > end {
            return Err(command_error("GetPrinterW", "invalid DEVMODE size".into()));
        }
        // dmSize 和 dmDriverExtra 均为 WORD：完整 DEVMODE 最大为 2 * u16::MAX 字节。
        // 保留该上界容量，DocumentProperties 合并时即使私有数据长度变化也不会越界。
        let mut mode = vec![0_u64; (2 * usize::from(u16::MAX)).div_ceil(8)];
        unsafe { ptr::copy_nonoverlapping(source.cast::<u8>(), mode.as_mut_ptr().cast(), length) };
        // 与 SumatraPDF NewPrinter 一样，用驱动当前默认配置初始化完整 DEVMODE。
        if unsafe {
            DocumentPropertiesW(
                ptr::null_mut(),
                printer,
                name.as_ptr(),
                mode.as_mut_ptr().cast(),
                ptr::null(),
                DM_OUT_BUFFER,
            )
        } != 1
        {
            return Err(command_error("DocumentPropertiesW", last_os_error()));
        }
        return Ok(mode);
    }
    Err(command_error(
        "GetPrinterW",
        "printer has no default DEVMODE".into(),
    ))
}

/// 在内存中让驱动验证完整 DEVMODE，不保存到打印机默认设置。
fn validate_devmode(printer: PRINTER_HANDLE, name: &[u16], mode: &mut [u64]) -> i32 {
    let pointer = mode.as_mut_ptr().cast::<DEVMODEW>();
    unsafe {
        DocumentPropertiesW(
            ptr::null_mut(),
            printer,
            name.as_ptr(),
            pointer,
            pointer,
            DM_IN_BUFFER | DM_OUT_BUFFER,
        )
    }
}

/// 读取等效打印配置的物理页面毫米尺寸，不创建打印任务。
fn physical_paper_size(name: &[u16], mode: &[u64]) -> PrintResult<(f64, f64)> {
    let dc = unsafe {
        CreateICW(
            ptr::null(),
            name.as_ptr(),
            ptr::null(),
            mode.as_ptr().cast(),
        )
    };
    if dc.is_null() {
        return Err(command_error("CreateICW", last_os_error()));
    }
    let width = unsafe { GetDeviceCaps(dc, PHYSICALWIDTH as i32) };
    let height = unsafe { GetDeviceCaps(dc, PHYSICALHEIGHT as i32) };
    let dpi_x = unsafe { GetDeviceCaps(dc, LOGPIXELSX as i32) };
    let dpi_y = unsafe { GetDeviceCaps(dc, LOGPIXELSY as i32) };
    unsafe { DeleteDC(dc) };
    if width <= 0 || height <= 0 || dpi_x <= 0 || dpi_y <= 0 {
        return Err(command_error("GetDeviceCaps", format!("invalid physical page: width={width}, height={height}, dpi_x={dpi_x}, dpi_y={dpi_y}")));
    }
    Ok((
        f64::from(width) * 25.4 / f64::from(dpi_x),
        f64::from(height) * 25.4 / f64::from(dpi_y),
    ))
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
    default: Option<bool>,
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
            is_default: value.default.unwrap_or(false),
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

/// 创建不打开独立控制台窗口的 Windows 子进程。
fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
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

#[cfg(test)]
mod tests {
    use super::parse_printers_json;

    #[test]
    fn parse_printers_json_accepts_null_default_flag() {
        let printers = parse_printers_json(
            r#"[{"Name":"Printer A","Default":null,"PortName":"USB001","Type":"Local","DeviceType":"Print","DriverName":"Driver"}]"#,
        )
        .unwrap();

        assert_eq!(printers.len(), 1);
        assert!(!printers[0].is_default);
        assert_eq!(printers[0].name, "Printer A");
    }
}
