use super::{
    common_label_papers, cups_media_option, paper_name, resolve_paper_for_print,
    submitted_at_rfc3339, PaperInfo, PrintBackend, PrintError, PrintOptions, PrintResult,
    PrintSubmission, PrintTrackingOutcome, PrinterInfo, PrinterMediaTypeInfo, PrinterTrayInfo,
    RawPrintOptions,
};
use std::{collections::HashMap, io::Write, path::Path, process::Command};

/// 基于 CUPS 命令行工具的打印后端。
#[derive(Debug, Clone, Copy)]
pub struct CupsPrintBackend {
    platform: CupsPlatform,
}

/// CUPS 后端当前运行的平台差异。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CupsPlatform {
    Macos,
    #[cfg(any(target_os = "linux", test))]
    Linux,
}

impl CupsPrintBackend {
    /// 创建 macOS CUPS 打印后端。
    pub fn macos() -> Self {
        Self {
            platform: CupsPlatform::Macos,
        }
    }

    /// 创建 Linux CUPS 打印后端。
    #[cfg(any(target_os = "linux", test))]
    pub fn linux() -> Self {
        Self {
            platform: CupsPlatform::Linux,
        }
    }

    fn backend_name(self) -> &'static str {
        match self.platform {
            CupsPlatform::Macos => "macos-cups",
            #[cfg(any(target_os = "linux", test))]
            CupsPlatform::Linux => "linux-cups",
        }
    }

    fn raw_backend_name(self) -> &'static str {
        match self.platform {
            CupsPlatform::Macos => "macos-cups-raw",
            #[cfg(any(target_os = "linux", test))]
            CupsPlatform::Linux => "linux-cups-raw",
        }
    }
}

impl PrintBackend for CupsPrintBackend {
    /// 列出 CUPS 目标，并标记当前默认打印机。
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
        let printers_output = run_command("lpstat", &["-e"])?;
        let default_printer = current_default_printer();
        let ports = current_printer_ports();

        let mut printers =
            parse_lpstat_destinations(&printers_output, default_printer.as_deref(), &ports);
        for printer in &mut printers {
            printer.dpi = read_lpoptions(&printer.name)
                .ok()
                .and_then(|output| parse_lpoptions_dpi(&output));
        }

        Ok(printers)
    }

    /// 列出 CUPS 纸张选项；不可用时回退到常见标签纸尺寸。
    fn list_papers(&self, printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
        ensure_printer_exists(self, printer_name)?;

        let output = read_lpoptions(printer_name)?;

        let papers = parse_lpoptions_papers(&output);
        if papers.is_empty() {
            Ok(common_label_papers())
        } else {
            Ok(papers)
        }
    }

    /// 列出 CUPS 纸盒或进纸来源选项。
    fn list_trays(&self, printer_name: &str) -> PrintResult<Vec<PrinterTrayInfo>> {
        ensure_printer_exists(self, printer_name)?;
        read_lpoptions(printer_name).map(|output| parse_lpoptions_trays(&output))
    }

    /// 列出 CUPS 介质类型选项。
    fn list_media_types(&self, printer_name: &str) -> PrintResult<Vec<PrinterMediaTypeInfo>> {
        ensure_printer_exists(self, printer_name)?;
        read_lpoptions(printer_name).map(|output| parse_lpoptions_media_types(&output))
    }

    /// 使用明确的份数和介质设置把 PDF 发送给 CUPS。
    fn print_pdf(&self, path: &Path, options: &PrintOptions) -> PrintResult<PrintSubmission> {
        ensure_printer_exists(self, &options.printer_name)?;
        let paper = resolve_print_paper(self, options)?;

        let copies = options.copies.max(1).to_string();
        let media = format!("media={}", cups_media_option(&paper));
        let output = Command::new("lp")
            .arg("-d")
            .arg(&options.printer_name)
            .arg("-n")
            .arg(copies)
            .arg("-o")
            .arg(media)
            .arg(path)
            .output()
            .map_err(|error| command_error("lp", error.to_string()))?;

        if output.status.success() {
            let system_job_id = parse_lp_job_id(&String::from_utf8_lossy(&output.stdout));
            let tracking_supported = system_job_id.is_some();
            Ok(PrintSubmission {
                submitted_at: submitted_at_rfc3339(),
                backend: self.backend_name().to_string(),
                system_job_id,
                tracking_supported,
            })
        } else {
            Err(command_error(
                "lp",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ))
        }
    }

    /// 使用 CUPS raw 模式把原始打印指令提交给打印机。
    fn print_raw(&self, data: &[u8], options: &RawPrintOptions) -> PrintResult<PrintSubmission> {
        ensure_printer_exists(self, &options.printer_name)?;
        let path = temp_raw_path();
        write_raw_temp_file(&path, data)?;

        let output = Command::new("lp")
            .arg("-d")
            .arg(&options.printer_name)
            .arg("-o")
            .arg("raw")
            .arg(&path)
            .output()
            .map_err(|error| command_error("lp", error.to_string()));
        let _ = std::fs::remove_file(&path);
        let output = output?;

        if output.status.success() {
            let system_job_id = parse_lp_job_id(&String::from_utf8_lossy(&output.stdout));
            let tracking_supported = system_job_id.is_some();
            Ok(PrintSubmission {
                submitted_at: submitted_at_rfc3339(),
                backend: self.raw_backend_name().to_string(),
                system_job_id,
                tracking_supported,
            })
        } else {
            Err(command_error(
                "lp",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ))
        }
    }

    fn track_submission(
        &self,
        submission: &PrintSubmission,
        _options: &PrintOptions,
    ) -> PrintTrackingOutcome {
        track_cups_submission(submission)
    }

    fn track_raw_submission(
        &self,
        submission: &PrintSubmission,
        _options: &RawPrintOptions,
    ) -> PrintTrackingOutcome {
        track_cups_submission(submission)
    }
}

fn track_cups_submission(submission: &PrintSubmission) -> PrintTrackingOutcome {
    let Some(job_id) = submission.system_job_id.as_deref() else {
        return PrintTrackingOutcome::Unknown {
            message: "CUPS did not expose a print job id".to_string(),
        };
    };

    match Command::new("lpstat")
        .args(["-W", "completed", "-o"])
        .output()
    {
        Ok(output) if output.status.success() => {
            if completed_jobs_contains_job(&String::from_utf8_lossy(&output.stdout), job_id) {
                PrintTrackingOutcome::Completed {
                    message: "CUPS reports the print job as completed".to_string(),
                }
            } else {
                PrintTrackingOutcome::Unknown {
                    message: "CUPS job status was no longer available".to_string(),
                }
            }
        }
        Ok(output) => PrintTrackingOutcome::Unknown {
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        },
        Err(error) => PrintTrackingOutcome::Unknown {
            message: error.to_string(),
        },
    }
}

fn temp_raw_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("print-bridge-raw-{}.bin", uuid::Uuid::new_v4()))
}

fn write_raw_temp_file(path: &Path, data: &[u8]) -> PrintResult<()> {
    let mut file = std::fs::File::create(path)
        .map_err(|error| command_error("raw-temp-file", error.to_string()))?;
    file.write_all(data)
        .map_err(|error| command_error("raw-temp-file", error.to_string()))?;
    file.flush()
        .map_err(|error| command_error("raw-temp-file", error.to_string()))
}

fn parse_lp_job_id(output: &str) -> Option<String> {
    output
        .split_once("request id is ")
        .and_then(|(_, request)| request.split_whitespace().next())
        .map(|part| part.trim_end_matches('.').to_string())
}

fn completed_jobs_contains_job(output: &str, job_id: &str) -> bool {
    output.lines().any(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|token| token.trim_end_matches('.') == job_id)
    })
}

/// 把 `lpstat -e` 输出解析为打印机摘要。
fn parse_lpstat_destinations(
    output: &str,
    default_printer: Option<&str>,
    ports: &HashMap<String, String>,
) -> Vec<PrinterInfo> {
    output
        .lines()
        .filter_map(|line| {
            let name = line.trim();
            if name.is_empty() {
                return None;
            }

            let mut printer = PrinterInfo::new(name.to_string(), default_printer == Some(name));
            if let Some(port) = ports.get(name) {
                let (is_local, is_network, is_virtual) = classify_port(port);
                printer.port = Some(port.clone());
                printer.is_local = is_local;
                printer.is_network = is_network;
                printer.is_virtual = is_virtual;
            }

            Some(printer)
        })
        .collect()
}

/// 读取当前默认 CUPS 目标。
fn current_default_printer() -> Option<String> {
    run_command("lpstat", &["-d"])
        .ok()
        .and_then(|output| parse_default_destination(&output))
}

/// 读取 CUPS 目标和设备 URI 的映射。
fn current_printer_ports() -> HashMap<String, String> {
    run_command("lpstat", &["-v"])
        .ok()
        .map(|output| parse_lpstat_devices(&output))
        .unwrap_or_default()
}

/// 解析 `lpstat -v` 输出中的设备 URI。
fn parse_lpstat_devices(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let (prefix, port) = line.split_once(": ")?;
            let name = prefix.strip_prefix("device for ").unwrap_or(prefix).trim();
            if name.is_empty() || port.trim().is_empty() {
                None
            } else {
                Some((name.to_string(), port.trim().to_string()))
            }
        })
        .collect()
}

/// 根据 CUPS 设备 URI 做保守的本地/网络/虚拟分类。
fn classify_port(port: &str) -> (Option<bool>, Option<bool>, Option<bool>) {
    let lower = port.to_ascii_lowercase();
    let is_virtual = lower.starts_with("file:") || lower.contains("pdf") || lower.contains("fax");
    let is_network = lower.starts_with("ipp:")
        || lower.starts_with("ipps:")
        || lower.starts_with("http:")
        || lower.starts_with("https:")
        || lower.starts_with("socket:")
        || lower.starts_with("lpd:")
        || lower.starts_with("smb:")
        || lower.starts_with("dnssd:");
    let is_local =
        lower.starts_with("usb:") || lower.starts_with("serial:") || lower.starts_with("parallel:");

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

/// 解析本地化或英文的 `lpstat -d` 输出。
fn parse_default_destination(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, destination) = line.rsplit_once(':').or_else(|| line.rsplit_once('：'))?;
        let destination = destination.trim();
        if destination.is_empty() {
            None
        } else {
            Some(destination.to_string())
        }
    })
}

/// 从 `lpoptions -l` 输出中提取页面或介质选项。
fn parse_lpoptions_papers(output: &str) -> Vec<PaperInfo> {
    output
        .lines()
        .filter(|line| {
            let option = line.to_ascii_lowercase();
            option.starts_with("pagesize/") || option.starts_with("media/")
        })
        .flat_map(parse_paper_line)
        .collect()
}

/// 从 `lpoptions -l` 输出中提取纸盒或进纸来源。
fn parse_lpoptions_trays(output: &str) -> Vec<PrinterTrayInfo> {
    output
        .lines()
        .filter(|line| option_starts_with(line, &["inputslot/", "mediasource/"]))
        .flat_map(|line| parse_option_choices(line).into_iter())
        .map(|choice| PrinterTrayInfo {
            id: choice.id,
            name: choice.name,
        })
        .collect()
}

/// 从 `lpoptions -l` 输出中提取介质类型。
fn parse_lpoptions_media_types(output: &str) -> Vec<PrinterMediaTypeInfo> {
    output
        .lines()
        .filter(|line| option_starts_with(line, &["mediatype/", "mediaclass/"]))
        .flat_map(|line| parse_option_choices(line).into_iter())
        .map(|choice| PrinterMediaTypeInfo {
            id: choice.id,
            name: choice.name,
        })
        .collect()
}

/// 从 `lpoptions -l` 输出中提取当前或最高 DPI。
fn parse_lpoptions_dpi(output: &str) -> Option<u32> {
    output
        .lines()
        .filter(|line| option_starts_with(line, &["resolution/", "printerresolution/"]))
        .flat_map(parse_option_choices)
        .filter_map(|choice| parse_dpi_choice(&choice.id))
        .max()
}

fn option_starts_with(line: &str, prefixes: &[&str]) -> bool {
    let option = line.to_ascii_lowercase();
    prefixes.iter().any(|prefix| option.starts_with(prefix))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OptionChoice {
    id: String,
    name: String,
}

/// 把一行 CUPS/PPD 选项解析为通用 choices。
fn parse_option_choices(line: &str) -> Vec<OptionChoice> {
    let Some((_, choices)) = line.split_once(':') else {
        return Vec::new();
    };

    choices
        .split_whitespace()
        .filter_map(|choice| parse_option_choice(choice.trim_start_matches('*')))
        .collect()
}

fn parse_option_choice(choice: &str) -> Option<OptionChoice> {
    if choice.is_empty() {
        return None;
    }

    let (id, name) = choice.split_once('/').unwrap_or((choice, choice));

    Some(OptionChoice {
        id: id.to_string(),
        name: name.replace('_', " "),
    })
}

fn parse_dpi_choice(choice: &str) -> Option<u32> {
    let lower = choice.to_ascii_lowercase();
    let dpi_index = lower.find("dpi").unwrap_or(lower.len());
    lower[..dpi_index]
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse::<u32>().ok())
        .max()
}

/// 把一行 CUPS 选项解析为该行中的所有纸张选项。
fn parse_paper_line(line: &str) -> Vec<PaperInfo> {
    let Some((_, choices)) = line.split_once(':') else {
        return Vec::new();
    };

    choices
        .split_whitespace()
        .filter_map(|choice| parse_paper_choice(choice.trim_start_matches('*')))
        .collect()
}

/// 把一个 CUPS 介质 token 解析为毫米纸张尺寸。
fn parse_paper_choice(choice: &str) -> Option<PaperInfo> {
    if let Some((name, width_mm, height_mm)) = known_paper_size(choice) {
        return Some(PaperInfo {
            id: choice.to_string(),
            name: name.to_string(),
            width_mm,
            height_mm,
        });
    }

    let size = choice
        .strip_prefix("Custom.")
        .or_else(|| choice.strip_prefix("custom_"))
        .unwrap_or(choice);
    let (width_mm, height_mm) = parse_dimension_pair(size)?;

    Some(PaperInfo {
        id: choice.to_string(),
        name: paper_name(width_mm, height_mm),
        width_mm,
        height_mm,
    })
}

/// 识别 CUPS/PPD 中常见的命名纸张 token。
fn known_paper_size(choice: &str) -> Option<(&'static str, f64, f64)> {
    let token = normalized_media_token(choice);
    if media_token_has_part(&token, "a3") || media_token_has_part(&token, "a3jis") {
        Some(("A3", 297.0, 420.0))
    } else if media_token_has_part(&token, "a4") || media_token_has_part(&token, "a4jis") {
        Some(("A4", 210.0, 297.0))
    } else if media_token_has_part(&token, "a5") || media_token_has_part(&token, "a5jis") {
        Some(("A5", 148.0, 210.0))
    } else if media_token_has_part(&token, "a6") || media_token_has_part(&token, "a6jis") {
        Some(("A6", 105.0, 148.0))
    } else if media_token_has_part(&token, "b4jis") {
        Some(("B4", 257.0, 364.0))
    } else if media_token_has_part(&token, "b5jis") {
        Some(("B5", 182.0, 257.0))
    } else if media_token_has_part(&token, "b6jis") {
        Some(("B6", 128.0, 182.0))
    } else if media_token_has_part(&token, "letter") {
        Some(("Letter", 215.9, 279.4))
    } else if media_token_has_part(&token, "legal") {
        Some(("Legal", 215.9, 355.6))
    } else if media_token_has_part(&token, "tabloid") {
        Some(("Tabloid", 279.4, 431.8))
    } else if media_token_has_part(&token, "ledger") {
        Some(("Ledger", 431.8, 279.4))
    } else if media_token_has_part(&token, "executive") {
        Some(("Executive", 184.15, 266.7))
    } else {
        None
    }
}

fn normalized_media_token(choice: &str) -> String {
    choice
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn media_token_has_part(token: &str, part: &str) -> bool {
    token.split('_').any(|item| item == part)
}

/// 解析 `Custom.60x40mm`、`iso_a4_210x297mm` 或 `8.5x11` 这类尺寸 token。
fn parse_dimension_pair(size: &str) -> Option<(f64, f64)> {
    let normalized_size = size.replace("mmx", "x");
    let (size_without_unit, unit) = strip_unit_suffix(&normalized_size);
    let x_index = size_without_unit.rfind('x')?;
    let width = trailing_number(&size_without_unit[..x_index])?;
    let height = leading_number(&size_without_unit[x_index + 1..])?;
    let use_inches = unit == Some("in") || (unit.is_none() && width <= 20.0 && height <= 20.0);

    if use_inches {
        Some((round_mm(width * 25.4), round_mm(height * 25.4)))
    } else {
        Some((round_mm(width), round_mm(height)))
    }
}

fn round_mm(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn strip_unit_suffix(size: &str) -> (&str, Option<&'static str>) {
    let lower_size = size.to_ascii_lowercase();
    if lower_size.ends_with("mm") {
        (&size[..size.len() - 2], Some("mm"))
    } else if lower_size.ends_with("in") {
        (&size[..size.len() - 2], Some("in"))
    } else {
        (size, None)
    }
}

fn trailing_number(value: &str) -> Option<f64> {
    let start = value
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_ascii_digit() && *character != '.')
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);

    value[start..].parse().ok()
}

fn leading_number(value: &str) -> Option<f64> {
    let end = value
        .char_indices()
        .find(|(_, character)| !character.is_ascii_digit() && *character != '.')
        .map(|(index, _)| index)
        .unwrap_or(value.len());

    value[..end].parse().ok()
}

/// 在查询纸张或打印前确保打印机名称存在。
fn ensure_printer_exists(backend: &CupsPrintBackend, printer_name: &str) -> PrintResult<()> {
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

/// 根据打印机支持的纸张列表解析请求纸张。
fn resolve_print_paper(
    backend: &CupsPrintBackend,
    options: &PrintOptions,
) -> PrintResult<PaperInfo> {
    let papers = backend.list_papers(&options.printer_name)?;
    Ok(resolve_paper_for_print(&papers, &options.paper))
}

/// 为失败的平台命令构造 PrintError。
fn command_error(command: &str, message: String) -> PrintError {
    PrintError::CommandFailed {
        command: command.to_string(),
        message,
    }
}

/// 运行命令，并在命令成功退出时返回 stdout。
fn run_command(command: &str, args: &[&str]) -> PrintResult<String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|error| command_error(command, error.to_string()))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(command_error(
            command,
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// 读取指定打印机的 CUPS/PPD 选项。
fn read_lpoptions(printer_name: &str) -> PrintResult<String> {
    let output = Command::new("lpoptions")
        .args(["-p", printer_name, "-l"])
        .output()
        .map_err(|error| command_error("lpoptions", error.to_string()))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(PrintError::PrinterNotFound(printer_name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_port, completed_jobs_contains_job, parse_default_destination, parse_lp_job_id,
        parse_lpoptions_dpi, parse_lpoptions_media_types, parse_lpoptions_papers,
        parse_lpoptions_trays, parse_lpstat_destinations, parse_lpstat_devices, CupsPrintBackend,
    };
    use std::collections::HashMap;

    #[test]
    fn backend_names_follow_platform_variant() {
        assert_eq!(CupsPrintBackend::macos().backend_name(), "macos-cups");
        assert_eq!(
            CupsPrintBackend::macos().raw_backend_name(),
            "macos-cups-raw"
        );
        assert_eq!(CupsPrintBackend::linux().backend_name(), "linux-cups");
        assert_eq!(
            CupsPrintBackend::linux().raw_backend_name(),
            "linux-cups-raw"
        );
    }

    #[test]
    fn parses_lpstat_destinations_and_marks_default_printer() {
        let printers = parse_lpstat_destinations(
            "HP_LaserJet_Professional_M1136_MFP\nKONICA_MINOLTA_C364e\n",
            Some("HP_LaserJet_Professional_M1136_MFP"),
            &HashMap::from([(
                "HP_LaserJet_Professional_M1136_MFP".to_string(),
                "usb://HP/LaserJet".to_string(),
            )]),
        );

        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].name, "HP_LaserJet_Professional_M1136_MFP");
        assert!(printers[0].is_default);
        assert_eq!(printers[0].port.as_deref(), Some("usb://HP/LaserJet"));
        assert_eq!(printers[0].is_local, Some(true));
        assert_eq!(printers[0].is_network, Some(false));
        assert_eq!(printers[0].is_virtual, Some(false));
        assert_eq!(printers[1].name, "KONICA_MINOLTA_C364e");
        assert!(!printers[1].is_default);
    }

    #[test]
    fn parses_lpstat_devices() {
        let ports = parse_lpstat_devices(
            "device for Label_Printer: socket://192.168.1.20\ndevice for PDF: file:///tmp/output.pdf\n",
        );

        assert_eq!(
            ports.get("Label_Printer").map(String::as_str),
            Some("socket://192.168.1.20")
        );
        assert_eq!(
            ports.get("PDF").map(String::as_str),
            Some("file:///tmp/output.pdf")
        );
    }

    #[test]
    fn classifies_common_printer_ports() {
        assert_eq!(
            classify_port("usb://Zebra/ZD421"),
            (Some(true), Some(false), Some(false))
        );
        assert_eq!(
            classify_port("ipp://printer.local/ipp/print"),
            (Some(false), Some(true), Some(false))
        );
        assert_eq!(
            classify_port("file:///tmp/output.pdf"),
            (Some(false), Some(false), Some(true))
        );
    }

    #[test]
    fn parses_localized_default_destination() {
        assert_eq!(
            parse_default_destination("系统默认目的位置：HP_LaserJet_Professional_M1136_MFP\n")
                .as_deref(),
            Some("HP_LaserJet_Professional_M1136_MFP")
        );
    }

    #[test]
    fn parses_english_default_destination() {
        assert_eq!(
            parse_default_destination(
                "system default destination: HP_LaserJet_Professional_M1136_MFP\n"
            )
            .as_deref(),
            Some("HP_LaserJet_Professional_M1136_MFP")
        );
    }

    #[test]
    fn parses_lp_job_id_from_submission_output() {
        assert_eq!(
            parse_lp_job_id("request id is HP_LaserJet-42 (1 file(s))\n").as_deref(),
            Some("HP_LaserJet-42")
        );
        assert_eq!(
            parse_lp_job_id("request id is label-printer-123.\n").as_deref(),
            Some("label-printer-123")
        );
        assert_eq!(
            parse_lp_job_id("warning cups-2 token\nrequest id is label-printer-123.\n").as_deref(),
            Some("label-printer-123")
        );
        assert_eq!(parse_lp_job_id("lp output without id"), None);
    }

    #[test]
    fn completed_jobs_match_exact_job_tokens() {
        let output = "\
Printer-10 user 1024 Mon Jul  6 00:00:00 2026
Printer-123 user 1024 Mon Jul  6 00:00:00 2026
Printer-2. user 1024 Mon Jul  6 00:00:00 2026
";

        assert!(!completed_jobs_contains_job(output, "Printer-1"));
        assert!(completed_jobs_contains_job(output, "Printer-10"));
        assert!(completed_jobs_contains_job(output, "Printer-2"));
    }

    #[test]
    fn parses_named_standard_papers_from_cups_options() {
        let papers = parse_lpoptions_papers(
            "PageSize/Page Size: *Letter A4 iso_a5_148x210mm na_legal_8.5x14in\n",
        );

        assert_paper(&papers, "Letter", 215.9, 279.4);
        assert_paper(&papers, "A4", 210.0, 297.0);
        assert_paper(&papers, "A5", 148.0, 210.0);
        assert_paper(&papers, "Legal", 215.9, 355.6);
    }

    #[test]
    fn parses_inches_and_millimeters_from_cups_size_tokens() {
        let papers = parse_lpoptions_papers(
            "PageSize/Page Size: *8.5x11 12x18 Custom.60x40mm iso_a4_210x297mm\n",
        );

        assert_paper(&papers, "215.9 x 279.4 mm", 215.9, 279.4);
        assert_paper(&papers, "304.8 x 457.2 mm", 304.8, 457.2);
        assert_paper(&papers, "60 x 40 mm", 60.0, 40.0);
        assert_paper(&papers, "A4", 210.0, 297.0);
    }

    #[test]
    fn parses_konica_jis_page_size_tokens() {
        let papers = parse_lpoptions_papers(
            "PageSize/Paper Size: A3JIS *A4JIS A5JIS A6JIS B4JIS B5JIS B6JIS 220mmx330mm 8.5x11 A4Wide\n",
        );

        assert_paper(&papers, "A3", 297.0, 420.0);
        assert_paper(&papers, "A4", 210.0, 297.0);
        assert_paper(&papers, "A5", 148.0, 210.0);
        assert_paper(&papers, "A6", 105.0, 148.0);
        assert_paper(&papers, "B4", 257.0, 364.0);
        assert_paper(&papers, "B5", 182.0, 257.0);
        assert_paper(&papers, "B6", 128.0, 182.0);
        assert_paper(&papers, "220 x 330 mm", 220.0, 330.0);
        assert_paper(&papers, "215.9 x 279.4 mm", 215.9, 279.4);
    }

    #[test]
    fn parses_trays_media_types_and_dpi_from_cups_options() {
        let output = "\
InputSlot/Media Source: *Auto Tray1/Tray_1 Manual/Manual_Feed
MediaType/Media Type: *Stationery Labels/Labels
Resolution/Output Resolution: *203dpi 300x300dpi 600dpi
";

        let trays = parse_lpoptions_trays(output);
        assert_eq!(trays[0].id, "Auto");
        assert_eq!(trays[1].name, "Tray 1");
        assert_eq!(trays[2].name, "Manual Feed");

        let media_types = parse_lpoptions_media_types(output);
        assert_eq!(media_types[0].id, "Stationery");
        assert_eq!(media_types[1].name, "Labels");

        assert_eq!(parse_lpoptions_dpi(output), Some(600));
    }

    fn assert_paper(papers: &[super::PaperInfo], name: &str, width_mm: f64, height_mm: f64) {
        let paper = papers
            .iter()
            .find(|paper| paper.name == name)
            .unwrap_or_else(|| panic!("missing paper {name} in {papers:?}"));

        assert_close(paper.width_mm, width_mm);
        assert_close(paper.height_mm, height_mm);
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.01,
            "expected {expected}, got {actual}"
        );
    }
}
