use crate::protocol::SupportedFormat;
use std::{
    ffi::OsString,
    fs,
    future::Future,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};
use thiserror::Error;
use tokio::{io::AsyncReadExt, process::Command};
use zip::{result::ZipError, ZipArchive};

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod libreoffice;

#[cfg(target_os = "windows")]
mod windows;

pub(crate) const OFFICE_CONVERSION_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_CONVERTER_MESSAGE_CHARS: usize = 2_048;
const TIMEOUT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const CONVERTER_EXIT_GRACE: Duration = Duration::from_secs(5);

/// 可转换为 PDF 的 Office 文档格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeFormat {
    Docx,
    Xlsx,
    Pptx,
}

impl OfficeFormat {
    /// 返回本机 Office 软件识别输入格式所需的扩展名。
    fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
        }
    }
}

/// Office 文档检测或转换为 PDF 时返回的错误。
#[derive(Debug, Error)]
pub enum OfficeConvertError {
    #[error("unsupported office format")]
    UnsupportedFormat,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("office converter unavailable: {converter}")]
    ConverterUnavailable { converter: &'static str },
    #[error("office conversion timed out after {seconds} seconds: {converter}")]
    TimedOut {
        converter: &'static str,
        seconds: u64,
    },
    #[error("office conversion failed via {converter}: {message}")]
    CommandFailed {
        converter: &'static str,
        message: String,
    },
    #[error("office converter produced an invalid PDF: {converter}")]
    InvalidPdf { converter: &'static str },
}

/// 把协议中的支持格式映射为 Office 转换格式。
pub fn office_format_from_supported(format: SupportedFormat) -> Option<OfficeFormat> {
    match format {
        SupportedFormat::Docx => Some(OfficeFormat::Docx),
        SupportedFormat::Xlsx => Some(OfficeFormat::Xlsx),
        SupportedFormat::Pptx => Some(OfficeFormat::Pptx),
        _ => None,
    }
}

/// 根据 OOXML 容器内容检测 Office 文档格式。
pub fn detect_office_format(path: &Path) -> Result<Option<OfficeFormat>, OfficeConvertError> {
    let file = fs::File::open(path)?;
    let mut archive = match ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(ZipError::InvalidArchive(_)) => return Ok(None),
        Err(ZipError::UnsupportedArchive(_)) => return Ok(None),
        Err(ZipError::Io(error)) => return Err(OfficeConvertError::Io(error)),
        Err(_) => return Ok(None),
    };

    if archive.by_name("word/document.xml").is_ok() {
        return Ok(Some(OfficeFormat::Docx));
    }
    if archive.by_name("xl/workbook.xml").is_ok() {
        return Ok(Some(OfficeFormat::Xlsx));
    }
    if archive.by_name("ppt/presentation.xml").is_ok() {
        return Ok(Some(OfficeFormat::Pptx));
    }

    Ok(None)
}

/// 把 Office 文档转换为 PDF 并写入指定路径。
pub async fn office_to_pdf(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
) -> Result<(), OfficeConvertError> {
    office_to_pdf_with_converter(
        input_path,
        format,
        output_path,
        |staged, format, output| async move {
            convert_with_current_backend(&staged, format, &output).await
        },
    )
    .await
}

/// 在隔离工作目录中执行一次可注入的 Office 转换。
async fn office_to_pdf_with_converter<F, Fut>(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
    converter: F,
) -> Result<(), OfficeConvertError>
where
    F: FnOnce(PathBuf, OfficeFormat, PathBuf) -> Fut,
    Fut: Future<Output = Result<&'static str, OfficeConvertError>>,
{
    let work_dir = input_path.with_extension("office-work");
    remove_dir_if_exists(&work_dir).await?;
    remove_file_if_exists(output_path).await?;
    tokio::fs::create_dir(&work_dir).await?;

    let staging_result: Result<PathBuf, OfficeConvertError> = async {
        let staged_path = staged_input_path(&work_dir, output_path, format)?;
        tokio::fs::copy(input_path, &staged_path).await?;
        Ok(staged_path)
    }
    .await;
    let staged_path = match staging_result {
        Ok(path) => path,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&work_dir).await;
            return Err(error);
        }
    };

    let conversion_result = match converter(staged_path, format, output_path.to_path_buf()).await {
        Ok(converter_name) => validate_pdf(output_path, converter_name),
        Err(error) => Err(error),
    };
    let cleanup_result = tokio::fs::remove_dir_all(&work_dir).await;

    match conversion_result {
        Ok(()) => {
            if let Err(error) = cleanup_result {
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(error.into());
            }
            Ok(())
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(output_path).await;
            let _ = cleanup_result;
            Err(error)
        }
    }
}

/// 生成与目标 PDF 同名且扩展名正确的暂存输入路径。
fn staged_input_path(
    work_dir: &Path,
    output_path: &Path,
    format: OfficeFormat,
) -> Result<PathBuf, OfficeConvertError> {
    let stem = output_path.file_stem().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "PDF output has no file name")
    })?;
    let mut file_name = OsString::from(stem);
    file_name.push(".");
    file_name.push(format.extension());
    Ok(work_dir.join(file_name))
}

/// 校验转换器输出存在、非空且具有 PDF 文件头。
fn validate_pdf(path: &Path, converter: &'static str) -> Result<(), OfficeConvertError> {
    let invalid = || OfficeConvertError::InvalidPdf { converter };
    let metadata = fs::metadata(path).map_err(|_| invalid())?;
    if metadata.len() <= 5 {
        return Err(invalid());
    }

    let mut file = fs::File::open(path).map_err(|_| invalid())?;
    let mut header = [0_u8; 5];
    file.read_exact(&mut header).map_err(|_| invalid())?;
    if &header != b"%PDF-" {
        return Err(invalid());
    }
    Ok(())
}

/// 删除文件，同时把不存在视为清理成功。
async fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// 删除目录，同时把不存在视为清理成功。
async fn remove_dir_if_exists(path: &Path) -> io::Result<()> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// 把转换器错误限制为任务历史允许的固定字符数。
fn truncate_message(value: &str) -> String {
    value.chars().take(MAX_CONVERTER_MESSAGE_CHARS).collect()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
/// 在 macOS/Linux 上调用本机 LibreOffice。
async fn convert_with_current_backend(
    input_path: &Path,
    _format: OfficeFormat,
    output_path: &Path,
) -> Result<&'static str, OfficeConvertError> {
    libreoffice::convert(input_path, output_path).await
}

#[cfg(target_os = "windows")]
/// 在 Windows 上调用本机 Microsoft Office。
async fn convert_with_current_backend(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
) -> Result<&'static str, OfficeConvertError> {
    windows::convert(input_path, format, output_path).await
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
/// 在非桌面平台报告 Office 转换器不可用。
async fn convert_with_current_backend(
    _input_path: &Path,
    _format: OfficeFormat,
    _output_path: &Path,
) -> Result<&'static str, OfficeConvertError> {
    Err(OfficeConvertError::ConverterUnavailable {
        converter: "Office converter",
    })
}

/// 在固定时限内执行 Office 转换子进程。
pub(crate) async fn execute_converter_command(
    command: Command,
    converter: &'static str,
    timeout: Duration,
) -> Result<Output, OfficeConvertError> {
    execute_converter_command_with_optional_cleanup(command, converter, timeout, None).await
}

#[cfg(any(target_os = "windows", test))]
/// 在超时前运行一次实例专属清理命令后执行 Office 转换子进程。
pub(crate) async fn execute_converter_command_with_timeout_cleanup(
    command: Command,
    converter: &'static str,
    timeout: Duration,
    cleanup_command: Command,
) -> Result<Output, OfficeConvertError> {
    execute_converter_command_with_optional_cleanup(
        command,
        converter,
        timeout,
        Some(cleanup_command),
    )
    .await
}

/// 在固定时限内执行 Office 转换子进程，并在超时后按需运行清理命令。
async fn execute_converter_command_with_optional_cleanup(
    mut command: Command,
    converter: &'static str,
    timeout: Duration,
    cleanup_command: Option<Command>,
) -> Result<Output, OfficeConvertError> {
    command.kill_on_drop(true);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(OfficeConvertError::ConverterUnavailable { converter });
        }
        Err(error) => return Err(error.into()),
    };
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(result) => result?,
        Err(_) => {
            if let Some(mut cleanup_command) = cleanup_command {
                cleanup_command.kill_on_drop(true);
                match tokio::time::timeout(TIMEOUT_CLEANUP_TIMEOUT, cleanup_command.status()).await
                {
                    Ok(Ok(status)) if status.success() => {
                        log::warn!("Office timeout cleanup completed for {converter}");
                    }
                    Ok(Ok(_)) => {
                        log::warn!("Office timeout cleanup skipped or failed for {converter}");
                    }
                    Ok(Err(_)) | Err(_) => {
                        log::warn!("Office timeout cleanup could not run for {converter}");
                    }
                }
            }

            if tokio::time::timeout(CONVERTER_EXIT_GRACE, child.wait())
                .await
                .is_err()
            {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            stdout_task.abort();
            stderr_task.abort();
            return Err(OfficeConvertError::TimedOut {
                converter,
                seconds: timeout.as_secs().max(1),
            });
        }
    };
    let stdout = stdout_task
        .await
        .map_err(|error| io::Error::other(error.to_string()))??;
    let stderr = stderr_task
        .await
        .map_err(|error| io::Error::other(error.to_string()))??;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// 把转换子进程输出压缩为可安全写入任务历史的错误。
pub(crate) fn command_failed(converter: &'static str, output: &Output) -> OfficeConvertError {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("process exited with {}", output.status)
    };
    OfficeConvertError::CommandFailed {
        converter,
        message: truncate_message(&message),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        detect_office_format, execute_converter_command,
        execute_converter_command_with_timeout_cleanup, office_format_from_supported,
        office_to_pdf_with_converter, truncate_message, OfficeConvertError, OfficeFormat,
        MAX_CONVERTER_MESSAGE_CHARS,
    };
    use crate::protocol::SupportedFormat;
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
        time::Duration,
    };
    use tokio::process::Command;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn maps_supported_formats_to_office_formats() {
        assert_eq!(
            office_format_from_supported(SupportedFormat::Docx),
            Some(OfficeFormat::Docx)
        );
        assert_eq!(
            office_format_from_supported(SupportedFormat::Xlsx),
            Some(OfficeFormat::Xlsx)
        );
        assert_eq!(
            office_format_from_supported(SupportedFormat::Pptx),
            Some(OfficeFormat::Pptx)
        );
        assert_eq!(office_format_from_supported(SupportedFormat::Pdf), None);
    }

    #[test]
    fn detects_minimal_ooxml_containers() {
        let docx = temp_path("sample.docx");
        let xlsx = temp_path("sample.xlsx");
        let pptx = temp_path("sample.pptx");
        let _ = fs::remove_file(&docx);
        let _ = fs::remove_file(&xlsx);
        let _ = fs::remove_file(&pptx);

        write_zip(&docx, &["word/document.xml"]);
        write_zip(&xlsx, &["xl/workbook.xml"]);
        write_zip(&pptx, &["ppt/presentation.xml"]);

        assert_eq!(
            detect_office_format(&docx).unwrap(),
            Some(OfficeFormat::Docx)
        );
        assert_eq!(
            detect_office_format(&xlsx).unwrap(),
            Some(OfficeFormat::Xlsx)
        );
        assert_eq!(
            detect_office_format(&pptx).unwrap(),
            Some(OfficeFormat::Pptx)
        );

        let _ = fs::remove_file(&docx);
        let _ = fs::remove_file(&xlsx);
        let _ = fs::remove_file(&pptx);
    }

    #[tokio::test]
    async fn stages_input_with_real_extension_and_cleans_work_dir() {
        let input = temp_path("stage-source.tmp");
        let output = input.with_extension("pdf");
        cleanup_paths(&[&input, &output, &input.with_extension("office-work")]);
        fs::write(&input, b"office-bytes").unwrap();
        fs::write(&output, b"stale-pdf").unwrap();

        office_to_pdf_with_converter(
            &input,
            OfficeFormat::Docx,
            &output,
            |staged, format, output| async move {
                assert_eq!(format, OfficeFormat::Docx);
                assert_eq!(
                    staged.extension().and_then(|value| value.to_str()),
                    Some("docx")
                );
                assert_eq!(fs::read(staged).unwrap(), b"office-bytes");
                assert!(!output.exists());
                fs::write(output, b"%PDF-1.7\n%%EOF").unwrap();
                Ok("Fake Office")
            },
        )
        .await
        .unwrap();

        assert!(output.exists());
        assert!(!input.with_extension("office-work").exists());
        cleanup_paths(&[&input, &output]);
    }

    #[tokio::test]
    async fn cleans_work_dir_when_staging_input_fails() {
        let input = temp_path("missing-stage-source.tmp");
        let output = input.with_extension("pdf");
        let work_dir = input.with_extension("office-work");
        cleanup_paths(&[&input, &output, &work_dir]);

        let error = office_to_pdf_with_converter(
            &input,
            OfficeFormat::Docx,
            &output,
            |_staged, _format, _output| async move { Ok("Fake Office") },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, OfficeConvertError::Io(_)));
        assert!(!work_dir.exists());
        cleanup_paths(&[&input, &output]);
    }

    #[tokio::test]
    async fn rejects_invalid_pdf_and_removes_partial_output() {
        let input = temp_path("invalid-pdf.tmp");
        let output = input.with_extension("pdf");
        cleanup_paths(&[&input, &output, &input.with_extension("office-work")]);
        fs::write(&input, b"office-bytes").unwrap();

        let error = office_to_pdf_with_converter(
            &input,
            OfficeFormat::Xlsx,
            &output,
            |_staged, _format, output| async move {
                fs::write(output, b"not-pdf").unwrap();
                Ok("Fake Office")
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            OfficeConvertError::InvalidPdf {
                converter: "Fake Office"
            }
        ));
        assert!(!output.exists());
        assert!(!input.with_extension("office-work").exists());
        cleanup_paths(&[&input]);
    }

    #[tokio::test]
    async fn rejects_missing_pdf_output() {
        let input = temp_path("missing-pdf.tmp");
        let output = input.with_extension("pdf");
        cleanup_paths(&[&input, &output, &input.with_extension("office-work")]);
        fs::write(&input, b"office-bytes").unwrap();

        let error = office_to_pdf_with_converter(
            &input,
            OfficeFormat::Docx,
            &output,
            |_staged, _format, _output| async move { Ok("Fake Office") },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, OfficeConvertError::InvalidPdf { .. }));
        assert!(!output.exists());
        cleanup_paths(&[&input]);
    }

    #[tokio::test]
    async fn rejects_empty_pdf_output() {
        let input = temp_path("empty-pdf.tmp");
        let output = input.with_extension("pdf");
        cleanup_paths(&[&input, &output, &input.with_extension("office-work")]);
        fs::write(&input, b"office-bytes").unwrap();

        let error = office_to_pdf_with_converter(
            &input,
            OfficeFormat::Xlsx,
            &output,
            |_staged, _format, output| async move {
                fs::write(output, b"").unwrap();
                Ok("Fake Office")
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, OfficeConvertError::InvalidPdf { .. }));
        assert!(!output.exists());
        cleanup_paths(&[&input]);
    }

    #[tokio::test]
    async fn removes_partial_output_when_converter_fails() {
        let input = temp_path("converter-failure.tmp");
        let output = input.with_extension("pdf");
        cleanup_paths(&[&input, &output, &input.with_extension("office-work")]);
        fs::write(&input, b"office-bytes").unwrap();

        let error = office_to_pdf_with_converter(
            &input,
            OfficeFormat::Pptx,
            &output,
            |_staged, _format, output| async move {
                fs::write(output, b"partial").unwrap();
                Err(OfficeConvertError::CommandFailed {
                    converter: "Fake Office",
                    message: "failed".to_string(),
                })
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, OfficeConvertError::CommandFailed { .. }));
        assert!(!output.exists());
        assert!(!input.with_extension("office-work").exists());
        cleanup_paths(&[&input]);
    }

    #[test]
    fn truncates_converter_error_output() {
        let message = truncate_message(&"x".repeat(MAX_CONVERTER_MESSAGE_CHARS + 20));

        assert_eq!(message.chars().count(), MAX_CONVERTER_MESSAGE_CHARS);
    }

    #[tokio::test]
    async fn converter_command_times_out() {
        #[cfg(unix)]
        let command = {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 1"]);
            command
        };
        #[cfg(windows)]
        let command = {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 1"]);
            command
        };

        let error = execute_converter_command(command, "Fake Office", Duration::from_millis(20))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            OfficeConvertError::TimedOut {
                converter: "Fake Office",
                seconds: 1
            }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_runs_cleanup_before_terminating_converter() {
        let marker = temp_path("timeout-cleanup-marker");
        cleanup_paths(&[&marker]);

        let mut converter = Command::new("sh");
        converter.args(["-c", "sleep 1"]);
        let mut cleanup = Command::new("sh");
        cleanup.args(["-c", &format!("printf cleaned > {}", marker.display())]);

        let error = execute_converter_command_with_timeout_cleanup(
            converter,
            "Fake Office",
            Duration::from_millis(20),
            cleanup,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, OfficeConvertError::TimedOut { .. }));
        assert_eq!(fs::read_to_string(&marker).unwrap(), "cleaned");
        cleanup_paths(&[&marker]);
    }

    #[tokio::test]
    async fn missing_converter_command_is_unavailable() {
        let command = Command::new("printbridge-missing-office-converter");
        let error = execute_converter_command(command, "Missing Office", Duration::from_secs(1))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            OfficeConvertError::ConverterUnavailable {
                converter: "Missing Office"
            }
        ));
    }

    fn write_zip(path: &Path, entries: &[&str]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for entry in entries {
            zip.start_file(entry, options).unwrap();
            zip.write_all(b"<xml/>").unwrap();
        }
        zip.finish().unwrap();
    }

    fn temp_path(file_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "print-bridge-office-test-{}-{file_name}",
            std::process::id()
        ))
    }

    fn cleanup_paths(paths: &[&Path]) {
        for path in paths {
            let _ = fs::remove_file(path);
            let _ = fs::remove_dir_all(path);
        }
    }
}
