use super::{
    common_label_papers, cups_media_option, paper_name, resolve_paper_for_print,
    submitted_at_rfc3339, PaperInfo, PrintBackend, PrintError, PrintOptions, PrintResult,
    PrintSubmission, PrintTrackingOutcome, PrinterInfo,
};
use std::{path::Path, process::Command};

/// 基于 CUPS 命令行工具的 macOS 打印后端。
pub struct MacosPrintBackend;

impl PrintBackend for MacosPrintBackend {
    /// 列出 CUPS 目标，并标记当前默认打印机。
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
        let printers_output = run_command("lpstat", &["-e"])?;
        let default_printer = current_default_printer();

        Ok(parse_lpstat_destinations(
            &printers_output,
            default_printer.as_deref(),
        ))
    }

    /// 列出 CUPS 纸张选项；不可用时回退到常见标签纸尺寸。
    fn list_papers(&self, printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
        ensure_printer_exists(self, printer_name)?;

        let output = Command::new("lpoptions")
            .args(["-p", printer_name, "-l"])
            .output()
            .map_err(|error| command_error("lpoptions", error.to_string()))?;

        if !output.status.success() {
            return Err(PrintError::PrinterNotFound(printer_name.to_string()));
        }

        let papers = parse_lpoptions_papers(&String::from_utf8_lossy(&output.stdout));
        if papers.is_empty() {
            Ok(common_label_papers())
        } else {
            Ok(papers)
        }
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
                backend: "macos-cups".to_string(),
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
fn parse_lpstat_destinations(output: &str, default_printer: Option<&str>) -> Vec<PrinterInfo> {
    output
        .lines()
        .filter_map(|line| {
            let name = line.trim();
            if name.is_empty() {
                return None;
            }

            Some(PrinterInfo {
                name: name.to_string(),
                is_default: default_printer == Some(name),
            })
        })
        .collect()
}

/// 读取当前默认 CUPS 目标。
fn current_default_printer() -> Option<String> {
    run_command("lpstat", &["-d"])
        .ok()
        .and_then(|output| parse_default_destination(&output))
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
fn ensure_printer_exists(backend: &MacosPrintBackend, printer_name: &str) -> PrintResult<()> {
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
    backend: &MacosPrintBackend,
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

#[cfg(test)]
mod tests {
    use super::{
        completed_jobs_contains_job, parse_default_destination, parse_lp_job_id,
        parse_lpoptions_papers, parse_lpstat_destinations,
    };

    #[test]
    fn parses_lpstat_destinations_and_marks_default_printer() {
        let printers = parse_lpstat_destinations(
            "HP_LaserJet_Professional_M1136_MFP\nKONICA_MINOLTA_C364e\n",
            Some("HP_LaserJet_Professional_M1136_MFP"),
        );

        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].name, "HP_LaserJet_Professional_M1136_MFP");
        assert!(printers[0].is_default);
        assert_eq!(printers[1].name, "KONICA_MINOLTA_C364e");
        assert!(!printers[1].is_default);
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
