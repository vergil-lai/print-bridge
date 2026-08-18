use super::{
    command_failed, execute_converter_command_with_timeout_cleanup, validate_pdf,
    OfficeCandidateStatus, OfficeConvertError, OfficeFormat, OFFICE_CONVERSION_TIMEOUT,
};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command as StdCommand, Output},
};
use tokio::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const UNAVAILABLE_MARKER: &str = "PRINTBRIDGE_CONVERTER_UNAVAILABLE:";

#[derive(Clone, Copy)]
enum Provider {
    Microsoft,
    Wps,
}

const POWERSHELL_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$format = $env:PRINTBRIDGE_OFFICE_FORMAT
$provider = $env:PRINTBRIDGE_OFFICE_PROVIDER
$inputPath = $env:PRINTBRIDGE_OFFICE_INPUT
$outputPath = $env:PRINTBRIDGE_OFFICE_OUTPUT
$recordPath = $env:PRINTBRIDGE_OFFICE_INSTANCE_RECORD
$app = $null
$document = $null
$ownsApp = $false

$converter = switch ("$provider/$format") {
    'microsoft/docx' { 'Microsoft Word' }
    'microsoft/xlsx' { 'Microsoft Excel' }
    'microsoft/pptx' { 'Microsoft PowerPoint' }
    'wps/docx' { 'WPS Writer' }
    'wps/xlsx' { 'WPS Spreadsheets' }
    'wps/pptx' { 'WPS Presentation' }
    default { throw "unsupported office provider or format: $provider/$format" }
}
$progId = switch ("$provider/$format") {
    'microsoft/docx' { 'Word.Application' }
    'microsoft/xlsx' { 'Excel.Application' }
    'microsoft/pptx' { 'PowerPoint.Application' }
    'wps/docx' { 'KWPS.Application' }
    'wps/xlsx' { 'KET.Application' }
    'wps/pptx' { 'KWPP.Application' }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class PrintBridgeOfficeWindow {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
}
'@

try {
    if ([string]::IsNullOrWhiteSpace($recordPath)) {
        throw 'office instance record path is required'
    }
    $existingInstances = @(Get-Process -ErrorAction SilentlyContinue | ForEach-Object {
        try { "$($_.Id):$($_.StartTime.ToUniversalTime().Ticks)" } catch {}
    })

    try {
        $app = New-Object -ComObject $progId
    } catch {
        [Console]::Error.WriteLine("PRINTBRIDGE_CONVERTER_UNAVAILABLE:COM activation failed")
        exit 2
    }

    try {
        [uint32]$officeProcessId = 0
        [void][PrintBridgeOfficeWindow]::GetWindowThreadProcessId(
            [IntPtr]$app.Hwnd,
            [ref]$officeProcessId
        )
        if ($officeProcessId -eq 0) {
            throw 'could not determine process id from application window'
        }
        $officeProcess = Get-Process -Id $officeProcessId -ErrorAction Stop
        $instanceKey = "$($officeProcess.Id):$($officeProcess.StartTime.ToUniversalTime().Ticks)"
        if ($existingInstances -contains $instanceKey) {
            throw 'application reused an existing user instance'
        }
        $record = [ordered]@{
            Nonce = [guid]::NewGuid().ToString('N')
            Pid = $officeProcess.Id
            StartTimeUtc = $officeProcess.StartTime.ToUniversalTime().Ticks
            ProcessName = $officeProcess.ProcessName
            ScriptPid = $PID
        }
        $temporaryRecord = "$recordPath.tmp"
        $record | ConvertTo-Json -Compress | Set-Content -LiteralPath $temporaryRecord -Encoding utf8 -NoNewline
        Move-Item -Force -LiteralPath $temporaryRecord -Destination $recordPath
        $ownsApp = $true
    } catch {
        [Console]::Error.WriteLine("PRINTBRIDGE_CONVERTER_UNAVAILABLE:cannot prove ownership of a new process")
        exit 2
    }

    try {
        $app.AutomationSecurity = 3
        switch ($format) {
            'docx' {
                $app.Visible = $false
                $app.DisplayAlerts = 0
                $app.Options.UpdateLinksAtOpen = $false
            }
            'xlsx' {
                $app.Visible = $false
                $app.DisplayAlerts = $false
                $app.AskToUpdateLinks = $false
            }
            'pptx' {
                $app.DisplayAlerts = 1
            }
        }
    } catch {
        [Console]::Error.WriteLine("PRINTBRIDGE_CONVERTER_UNAVAILABLE:required automation security controls unavailable")
        exit 2
    }

    switch ($format) {
        'docx' {
            $document = $app.Documents.Open($inputPath, $false, $true, $false)
            $document.ExportAsFixedFormat($outputPath, 17, $false)
        }
        'xlsx' {
            $document = $app.Workbooks.Open($inputPath, 0, $true)
            $document.ExportAsFixedFormat(0, $outputPath, 0, $true, $false)
        }
        'pptx' {
            $document = $app.Presentations.Open($inputPath, $true, $false, $false)
            $document.SaveAs($outputPath, 32)
        }
    }
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
} finally {
    try { if ($ownsApp -and $document -ne $null) { if ($format -eq 'pptx') { $document.Close() } else { $document.Close($false) } } } catch {}
    try { if ($ownsApp -and $app -ne $null) { $app.Quit() } } catch {}
    try { if ($document -ne $null) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($document) } } catch {}
    try { if ($app -ne $null) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } } catch {}
}
"#;

const TIMEOUT_CLEANUP_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'

try {
    $recordPath = $env:PRINTBRIDGE_OFFICE_INSTANCE_RECORD
    if ([string]::IsNullOrWhiteSpace($recordPath) -or -not (Test-Path -LiteralPath $recordPath)) {
        exit 0
    }

    $record = Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json -ErrorAction Stop
    $recordPid = [int]$record.Pid
    $recordStartTimeUtc = [int64]$record.StartTimeUtc
    $recordProcessName = [string]$record.ProcessName
    if ($recordPid -le 0 -or $recordStartTimeUtc -le 0 -or [string]::IsNullOrWhiteSpace($recordProcessName)) {
        exit 0
    }

    $process = Get-Process -Id $recordPid -ErrorAction Stop
    if ($process.ProcessName -eq $recordProcessName -and
        $process.StartTime.ToUniversalTime().Ticks -eq $recordStartTimeUtc) {
        Stop-Process -Id $process.Id -ErrorAction Stop
    }
} catch {}
"#;

/// 按 Microsoft Office、WPS Office、LibreOffice 的顺序转换 Office 文件。
pub(super) async fn convert(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
) -> Result<&'static str, OfficeConvertError> {
    let mut unavailable = Vec::new();

    for provider in [Provider::Microsoft, Provider::Wps] {
        match convert_with_provider(input_path, format, output_path, provider).await {
            Ok(converter) => return Ok(converter),
            Err(error @ OfficeConvertError::ConverterUnavailable { .. }) => {
                unavailable.push(error.to_string());
            }
            Err(error) => return Err(with_unavailable_context(error, &unavailable)),
        }
    }

    match super::libreoffice::convert(input_path, output_path).await {
        Ok(converter) => {
            validate_pdf(output_path, converter)
                .map_err(|error| with_unavailable_context(error, &unavailable))?;
            Ok(converter)
        }
        Err(error @ OfficeConvertError::ConverterUnavailable { .. }) => {
            unavailable.push(error.to_string());
            Err(OfficeConvertError::ConvertersUnavailable {
                attempts: unavailable.join("; "),
            })
        }
        Err(error) => Err(with_unavailable_context(error, &unavailable)),
    }
}

/// 返回 doctor 使用的被动转换器探测结果，不启动任何 Office 应用。
pub(super) fn candidate_statuses(format: OfficeFormat) -> Vec<OfficeCandidateStatus> {
    vec![
        OfficeCandidateStatus {
            name: converter_name(Provider::Microsoft, format),
            available: is_com_registered(prog_id(Provider::Microsoft, format)),
        },
        OfficeCandidateStatus {
            name: converter_name(Provider::Wps, format),
            available: is_com_registered(prog_id(Provider::Wps, format)),
        },
        OfficeCandidateStatus {
            name: "LibreOffice",
            available: super::libreoffice::find_libreoffice().is_some(),
        },
    ]
}

/// 使用指定的 Windows Office COM 提供方执行一次转换。
async fn convert_with_provider(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
    provider: Provider,
) -> Result<&'static str, OfficeConvertError> {
    let converter = converter_name(provider, format);
    let record_path = instance_record_path(input_path, provider);
    let command = build_command(input_path, format, output_path, &record_path, provider);
    let cleanup = build_cleanup_command(&record_path);
    let output = execute_converter_command_with_timeout_cleanup(
        command,
        converter,
        OFFICE_CONVERSION_TIMEOUT,
        cleanup,
    )
    .await?;
    if !output.status.success() {
        return Err(failure_from_output(&output, converter));
    }
    validate_pdf(output_path, converter)?;
    Ok(converter)
}

/// 给后续转换失败附加此前不可用的候选信息。
fn with_unavailable_context(
    error: OfficeConvertError,
    unavailable: &[String],
) -> OfficeConvertError {
    if unavailable.is_empty() {
        error
    } else {
        OfficeConvertError::FallbackFailed {
            source: Box::new(error),
            unavailable: unavailable.join("; "),
        }
    }
}

/// 返回指定提供方和格式的显示名称。
fn converter_name(provider: Provider, format: OfficeFormat) -> &'static str {
    match (provider, format) {
        (Provider::Microsoft, OfficeFormat::Docx) => "Microsoft Word",
        (Provider::Microsoft, OfficeFormat::Xlsx) => "Microsoft Excel",
        (Provider::Microsoft, OfficeFormat::Pptx) => "Microsoft PowerPoint",
        (Provider::Wps, OfficeFormat::Docx) => "WPS Writer",
        (Provider::Wps, OfficeFormat::Xlsx) => "WPS Spreadsheets",
        (Provider::Wps, OfficeFormat::Pptx) => "WPS Presentation",
    }
}

/// 返回指定提供方和格式的 COM ProgID。
fn prog_id(provider: Provider, format: OfficeFormat) -> &'static str {
    match (provider, format) {
        (Provider::Microsoft, OfficeFormat::Docx) => "Word.Application",
        (Provider::Microsoft, OfficeFormat::Xlsx) => "Excel.Application",
        (Provider::Microsoft, OfficeFormat::Pptx) => "PowerPoint.Application",
        (Provider::Wps, OfficeFormat::Docx) => "KWPS.Application",
        (Provider::Wps, OfficeFormat::Xlsx) => "KET.Application",
        (Provider::Wps, OfficeFormat::Pptx) => "KWPP.Application",
    }
}

/// 返回传给 PowerShell 的提供方名称。
fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Microsoft => "microsoft",
        Provider::Wps => "wps",
    }
}

/// 返回传给 PowerShell 的 Office 格式名称。
fn format_name(format: OfficeFormat) -> &'static str {
    match format {
        OfficeFormat::Docx => "docx",
        OfficeFormat::Xlsx => "xlsx",
        OfficeFormat::Pptx => "pptx",
    }
}

/// 返回当前暂存输入和提供方对应的实例所有权记录路径。
fn instance_record_path(input_path: &Path, provider: Provider) -> PathBuf {
    input_path.with_extension(format!("{}-instance.json", provider_name(provider)))
}

/// 构造通过环境变量传递路径的非交互 PowerShell 命令。
fn build_command(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
    record_path: &Path,
    provider: Provider,
) -> Command {
    let mut command = hidden_command("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        POWERSHELL_SCRIPT,
    ]);
    command.env("PRINTBRIDGE_OFFICE_FORMAT", format_name(format));
    command.env("PRINTBRIDGE_OFFICE_PROVIDER", provider_name(provider));
    command.env("PRINTBRIDGE_OFFICE_INPUT", input_path);
    command.env("PRINTBRIDGE_OFFICE_OUTPUT", output_path);
    command.env("PRINTBRIDGE_OFFICE_INSTANCE_RECORD", record_path);
    command
}

/// 构造仅清理已记录 Office 实例的非交互 PowerShell 命令。
fn build_cleanup_command(record_path: &Path) -> Command {
    let mut command = hidden_command("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        TIMEOUT_CLEANUP_SCRIPT,
    ]);
    command.env("PRINTBRIDGE_OFFICE_INSTANCE_RECORD", record_path);
    command
}

/// 创建不打开独立控制台窗口的 Windows 子进程。
fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// 被动检查 COM ProgID 是否已注册。
fn is_com_registered(prog_id: &str) -> bool {
    use std::os::windows::process::CommandExt;

    let mut command = StdCommand::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW).args([
        "query",
        &format!(r"HKCR\{prog_id}\CLSID"),
        "/ve",
    ]);
    command.output().is_ok_and(|output| output.status.success())
}

/// 把 PowerShell 约定的不可用退出码映射为领域错误。
fn failure_from_exit(
    code: Option<i32>,
    converter: &'static str,
    stderr: &[u8],
) -> Option<OfficeConvertError> {
    if code != Some(2) {
        return None;
    }
    let message = String::from_utf8_lossy(stderr);
    let reason = message
        .lines()
        .find_map(|line| line.trim().strip_prefix(UNAVAILABLE_MARKER))
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("COM converter unavailable")
        .to_string();
    Some(OfficeConvertError::ConverterUnavailable { converter, reason })
}

/// 保留 PowerShell 真实错误输出并映射失败类型。
fn failure_from_output(output: &Output, converter: &'static str) -> OfficeConvertError {
    if let Some(error) = failure_from_exit(output.status.code(), converter, &output.stderr) {
        return error;
    }
    command_failed(converter, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn selects_converter_and_prog_id_for_each_provider() {
        assert_eq!(
            converter_name(Provider::Microsoft, OfficeFormat::Docx),
            "Microsoft Word"
        );
        assert_eq!(
            converter_name(Provider::Wps, OfficeFormat::Xlsx),
            "WPS Spreadsheets"
        );
        assert_eq!(
            prog_id(Provider::Microsoft, OfficeFormat::Pptx),
            "PowerPoint.Application"
        );
        assert_eq!(
            prog_id(Provider::Wps, OfficeFormat::Docx),
            "KWPS.Application"
        );
        assert_eq!(
            prog_id(Provider::Wps, OfficeFormat::Xlsx),
            "KET.Application"
        );
        assert_eq!(
            prog_id(Provider::Wps, OfficeFormat::Pptx),
            "KWPP.Application"
        );
    }

    #[test]
    fn builds_noninteractive_wps_command_with_path_env_vars() {
        let input = Path::new(r"C:\Temp\input with space.docx");
        let output = Path::new(r"C:\Temp\output with space.pdf");
        let record = instance_record_path(input, Provider::Wps);
        let command = build_command(input, OfficeFormat::Docx, output, &record, Provider::Wps);
        let standard = command.as_std();
        let args: Vec<_> = standard
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let envs: std::collections::HashMap<_, _> = standard
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value.to_owned())))
            .collect();

        assert!(args.contains(&"-NoProfile".to_string()));
        assert!(args.contains(&"-NonInteractive".to_string()));
        assert_eq!(
            envs.get(OsStr::new("PRINTBRIDGE_OFFICE_PROVIDER"))
                .unwrap()
                .as_os_str(),
            OsStr::new("wps")
        );
        assert_eq!(
            envs.get(OsStr::new("PRINTBRIDGE_OFFICE_INPUT")).unwrap(),
            input.as_os_str()
        );
        assert_eq!(
            envs.get(OsStr::new("PRINTBRIDGE_OFFICE_OUTPUT")).unwrap(),
            output.as_os_str()
        );
        assert!(!POWERSHELL_SCRIPT.contains(&input.display().to_string()));
    }

    #[test]
    fn scripts_record_and_verify_only_new_owned_instances() {
        assert!(POWERSHELL_SCRIPT.contains("GetWindowThreadProcessId"));
        assert!(POWERSHELL_SCRIPT.contains("existingInstances -contains"));
        assert!(POWERSHELL_SCRIPT.contains("StartTimeUtc"));
        assert!(POWERSHELL_SCRIPT.contains("ProcessName = $officeProcess.ProcessName"));
        assert!(TIMEOUT_CLEANUP_SCRIPT.contains("Get-Process -Id"));
        assert!(TIMEOUT_CLEANUP_SCRIPT.contains("Stop-Process -Id"));
        assert!(!TIMEOUT_CLEANUP_SCRIPT.contains("taskkill"));
        assert!(!TIMEOUT_CLEANUP_SCRIPT.contains("GetActiveObject"));
    }

    #[test]
    fn maps_exit_code_two_to_converter_unavailable_with_reason() {
        let stderr = b"PRINTBRIDGE_CONVERTER_UNAVAILABLE:COM activation failed\n";
        let error = failure_from_exit(Some(2), "WPS Writer", stderr).unwrap();
        assert!(matches!(
            error,
            OfficeConvertError::ConverterUnavailable {
                converter: "WPS Writer",
                ref reason
            } if reason == "COM activation failed"
        ));
    }

    #[test]
    fn powershell_script_forces_security_and_closes_only_owned_apps() {
        assert!(POWERSHELL_SCRIPT.contains("AutomationSecurity = 3"));
        assert!(POWERSHELL_SCRIPT.contains("UpdateLinksAtOpen = $false"));
        assert!(POWERSHELL_SCRIPT.contains("AskToUpdateLinks = $false"));
        assert!(POWERSHELL_SCRIPT.contains("$ownsApp -and $document"));
        assert!(POWERSHELL_SCRIPT.contains("$ownsApp -and $app"));
    }
}
