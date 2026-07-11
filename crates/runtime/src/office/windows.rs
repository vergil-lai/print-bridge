use super::{
    command_failed, execute_converter_command_with_timeout_cleanup, OfficeConvertError,
    OfficeFormat, OFFICE_CONVERSION_TIMEOUT,
};
use std::{
    path::{Path, PathBuf},
    process::Output,
};
use tokio::process::Command;

const POWERSHELL_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$format = $env:PRINTBRIDGE_OFFICE_FORMAT
$inputPath = $env:PRINTBRIDGE_OFFICE_INPUT
$outputPath = $env:PRINTBRIDGE_OFFICE_OUTPUT
$recordPath = $env:PRINTBRIDGE_OFFICE_INSTANCE_RECORD
$app = $null
$document = $null
$ownsApp = $false
$converter = switch ($format) {
    'docx' { 'Microsoft Word' }
    'xlsx' { 'Microsoft Excel' }
    'pptx' { 'Microsoft PowerPoint' }
    default { throw "unsupported office format: $format" }
}
$progId = switch ($format) {
    'docx' { 'Word.Application' }
    'xlsx' { 'Excel.Application' }
    'pptx' { 'PowerPoint.Application' }
}
$processName = $env:PRINTBRIDGE_OFFICE_PROCESS_NAME

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
    $existingInstances = @(Get-Process -Name $processName -ErrorAction SilentlyContinue |
        ForEach-Object { "$($_.Id):$($_.StartTime.ToUniversalTime().Ticks)" })

    try {
        $app = New-Object -ComObject $progId
    } catch {
        [Console]::Error.WriteLine("PRINTBRIDGE_CONVERTER_UNAVAILABLE:$converter")
        exit 2
    }

    [uint32]$officeProcessId = 0
    [void][PrintBridgeOfficeWindow]::GetWindowThreadProcessId(
        [IntPtr]$app.Hwnd,
        [ref]$officeProcessId
    )
    if ($officeProcessId -eq 0) {
        throw 'could not determine Office process id from application window'
    }
    $officeProcess = Get-Process -Id $officeProcessId -ErrorAction Stop
    $instanceKey = "$($officeProcess.Id):$($officeProcess.StartTime.ToUniversalTime().Ticks)"
    if ($officeProcess.ProcessName -ne $processName -or $existingInstances -contains $instanceKey) {
        throw 'Office application is not a newly created owned instance'
    }
    $record = [ordered]@{
        Nonce = [guid]::NewGuid().ToString('N')
        Pid = $officeProcess.Id
        StartTimeUtc = $officeProcess.StartTime.ToUniversalTime().Ticks
        ProcessName = $processName
        ScriptPid = $PID
    }
    $temporaryRecord = "$($env:PRINTBRIDGE_OFFICE_INSTANCE_RECORD).tmp"
    $record | ConvertTo-Json -Compress | Set-Content -LiteralPath $temporaryRecord -Encoding utf8 -NoNewline
    Move-Item -Force -LiteralPath $temporaryRecord -Destination $env:PRINTBRIDGE_OFFICE_INSTANCE_RECORD
    $ownsApp = $true

    $app.AutomationSecurity = 3
    switch ($format) {
        'docx' {
            $app.Visible = $false
            $app.DisplayAlerts = 0
            $app.Options.UpdateLinksAtOpen = $false
            $document = $app.Documents.Open($inputPath, $false, $true, $false)
            $document.ExportAsFixedFormat($outputPath, 17, $false)
        }
        'xlsx' {
            $app.Visible = $false
            $app.DisplayAlerts = $false
            $app.AskToUpdateLinks = $false
            $document = $app.Workbooks.Open($inputPath, 0, $true)
            $document.ExportAsFixedFormat(0, $outputPath, 0, $true, $false)
        }
        'pptx' {
            $app.DisplayAlerts = 1
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

/// 使用本机 Microsoft Office 把 Office 文件转换为 PDF。
pub(super) async fn convert(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
) -> Result<&'static str, OfficeConvertError> {
    let converter = converter_name(format);
    let record_path = instance_record_path(input_path);
    let command = build_command(input_path, format, output_path, &record_path);
    let cleanup = build_cleanup_command(&record_path);
    let output = execute_converter_command_with_timeout_cleanup(
        command,
        converter,
        OFFICE_CONVERSION_TIMEOUT,
        cleanup,
    )
    .await?;
    if output.status.success() {
        return Ok(converter);
    }
    Err(failure_from_output(&output, converter))
}

/// 返回处理指定格式的 Microsoft Office 应用名称。
fn converter_name(format: OfficeFormat) -> &'static str {
    match format {
        OfficeFormat::Docx => "Microsoft Word",
        OfficeFormat::Xlsx => "Microsoft Excel",
        OfficeFormat::Pptx => "Microsoft PowerPoint",
    }
}

/// 返回指定格式对应的 Windows Office 进程名。
fn office_process_name(format: OfficeFormat) -> &'static str {
    match format {
        OfficeFormat::Docx => "WINWORD",
        OfficeFormat::Xlsx => "EXCEL",
        OfficeFormat::Pptx => "POWERPNT",
    }
}

/// 返回当前暂存输入对应的实例所有权记录路径。
fn instance_record_path(input_path: &Path) -> PathBuf {
    input_path.with_extension("office-instance.json")
}

/// 返回传给 PowerShell 的 Office 格式名称。
fn format_name(format: OfficeFormat) -> &'static str {
    match format {
        OfficeFormat::Docx => "docx",
        OfficeFormat::Xlsx => "xlsx",
        OfficeFormat::Pptx => "pptx",
    }
}

/// 构造通过环境变量传递路径的非交互 PowerShell 命令。
fn build_command(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
    record_path: &Path,
) -> Command {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        POWERSHELL_SCRIPT,
    ]);
    command.env("PRINTBRIDGE_OFFICE_FORMAT", format_name(format));
    command.env(
        "PRINTBRIDGE_OFFICE_PROCESS_NAME",
        office_process_name(format),
    );
    command.env("PRINTBRIDGE_OFFICE_INPUT", input_path);
    command.env("PRINTBRIDGE_OFFICE_OUTPUT", output_path);
    command.env("PRINTBRIDGE_OFFICE_INSTANCE_RECORD", record_path);
    command
}

/// 构造仅清理已记录 Office 实例的非交互 PowerShell 命令。
fn build_cleanup_command(record_path: &Path) -> Command {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        TIMEOUT_CLEANUP_SCRIPT,
    ]);
    command.env("PRINTBRIDGE_OFFICE_INSTANCE_RECORD", record_path);
    command
}

/// 把 PowerShell 约定的不可用退出码映射为领域错误。
fn failure_from_exit(code: Option<i32>, converter: &'static str) -> Option<OfficeConvertError> {
    if code == Some(2) {
        Some(OfficeConvertError::ConverterUnavailable { converter })
    } else {
        None
    }
}

/// 保留 PowerShell 真实错误输出并映射失败类型。
fn failure_from_output(output: &Output, converter: &'static str) -> OfficeConvertError {
    if let Some(error) = failure_from_exit(output.status.code(), converter) {
        return error;
    }
    command_failed(converter, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn selects_converter_for_each_format() {
        assert_eq!(converter_name(OfficeFormat::Docx), "Microsoft Word");
        assert_eq!(converter_name(OfficeFormat::Xlsx), "Microsoft Excel");
        assert_eq!(converter_name(OfficeFormat::Pptx), "Microsoft PowerPoint");
        assert_eq!(office_process_name(OfficeFormat::Docx), "WINWORD");
        assert_eq!(office_process_name(OfficeFormat::Xlsx), "EXCEL");
        assert_eq!(office_process_name(OfficeFormat::Pptx), "POWERPNT");
    }

    #[test]
    fn builds_noninteractive_powershell_command_with_path_env_vars() {
        let input = Path::new(r"C:\Temp\input with space.docx");
        let output = Path::new(r"C:\Temp\output with space.pdf");
        let record = instance_record_path(input);
        let command = build_command(input, OfficeFormat::Docx, output, &record);
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
        assert!(args.contains(&POWERSHELL_SCRIPT.to_string()));
        assert_eq!(
            envs.get(OsStr::new("PRINTBRIDGE_OFFICE_FORMAT"))
                .unwrap()
                .as_os_str(),
            OsStr::new("docx")
        );
        assert_eq!(
            envs.get(OsStr::new("PRINTBRIDGE_OFFICE_INPUT")).unwrap(),
            input.as_os_str()
        );
        assert_eq!(
            envs.get(OsStr::new("PRINTBRIDGE_OFFICE_OUTPUT")).unwrap(),
            output.as_os_str()
        );
        assert_eq!(
            envs.get(OsStr::new("PRINTBRIDGE_OFFICE_INSTANCE_RECORD"))
                .unwrap(),
            record.as_os_str()
        );
        assert!(!POWERSHELL_SCRIPT.contains(&input.display().to_string()));
    }

    #[test]
    fn conversion_and_cleanup_commands_pass_record_path_via_environment() {
        let input = Path::new(r"C:\Temp\input.docx");
        let output = Path::new(r"C:\Temp\output.pdf");
        let record = instance_record_path(input);
        let command = build_command(input, OfficeFormat::Docx, output, &record);
        let cleanup = build_cleanup_command(&record);

        assert_eq!(
            command.as_std().get_envs().find_map(|(key, value)| {
                (key == OsStr::new("PRINTBRIDGE_OFFICE_INSTANCE_RECORD")).then(|| value.unwrap())
            }),
            Some(record.as_os_str()),
        );
        assert_eq!(
            cleanup.as_std().get_envs().find_map(|(key, value)| {
                (key == OsStr::new("PRINTBRIDGE_OFFICE_INSTANCE_RECORD")).then(|| value.unwrap())
            }),
            Some(record.as_os_str()),
        );
    }

    #[test]
    fn scripts_record_and_verify_only_owned_office_instances() {
        assert!(POWERSHELL_SCRIPT.contains("GetWindowThreadProcessId"));
        assert!(POWERSHELL_SCRIPT.contains("StartTimeUtc"));
        assert!(POWERSHELL_SCRIPT.contains("ConvertTo-Json"));
        assert!(POWERSHELL_SCRIPT.contains("Move-Item -Force"));
        assert!(TIMEOUT_CLEANUP_SCRIPT.contains("Get-Process -Id"));
        assert!(TIMEOUT_CLEANUP_SCRIPT.contains("Stop-Process -Id"));
        assert!(!TIMEOUT_CLEANUP_SCRIPT.contains("taskkill"));
        assert!(!TIMEOUT_CLEANUP_SCRIPT.contains("GetActiveObject"));
    }

    #[test]
    fn maps_exit_code_two_to_converter_unavailable() {
        let error = failure_from_exit(Some(2), "Microsoft Word").unwrap();
        assert!(matches!(
            error,
            OfficeConvertError::ConverterUnavailable {
                converter: "Microsoft Word"
            }
        ));
    }

    #[test]
    fn powershell_script_forces_macro_security_and_closes_apps() {
        assert!(POWERSHELL_SCRIPT.contains("AutomationSecurity = 3"));
        assert!(POWERSHELL_SCRIPT.contains("UpdateLinksAtOpen = $false"));
        assert!(POWERSHELL_SCRIPT.contains("AskToUpdateLinks = $false"));
        assert!(POWERSHELL_SCRIPT.contains("$document.Close"));
        assert!(POWERSHELL_SCRIPT.contains("$app.Quit"));
    }
}
