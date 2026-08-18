use super::{
    command_failed, execute_converter_command, OfficeConvertError, OFFICE_CONVERSION_TIMEOUT,
};
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
#[cfg(target_os = "windows")]
use std::{os::windows::process::CommandExt, process::Command as StdCommand};
use tokio::process::Command;
use url::Url;

const CONVERTER: &str = "LibreOffice";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "macos")]
const MACOS_SOFFICE: &str = "/Applications/LibreOffice.app/Contents/MacOS/soffice";
const MACRO_SECURITY_CONFIG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry">
  <item oor:path="/org.openoffice.Office.Common/Security/Scripting">
    <prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop>
  </item>
</oor:items>
"#;

/// 使用隔离配置调用 LibreOffice 把 Office 文件转换为 PDF。
pub(super) async fn convert(
    input_path: &Path,
    output_path: &Path,
) -> Result<&'static str, OfficeConvertError> {
    let executable = find_libreoffice().ok_or(OfficeConvertError::ConverterUnavailable {
        converter: CONVERTER,
        reason: "executable not found".to_string(),
    })?;
    let work_dir = input_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "staged input has no parent",
        )
    })?;
    let profile_dir = work_dir.join("libreoffice-profile");
    write_macro_security_profile(&profile_dir).await?;
    let profile_url = Url::from_directory_path(&profile_dir).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid LibreOffice profile path",
        )
    })?;
    let output_dir = output_path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "PDF output has no parent")
    })?;

    let command = build_command(&executable, profile_url.as_str(), input_path, output_dir);
    let output = execute_converter_command(command, CONVERTER, OFFICE_CONVERSION_TIMEOUT).await?;
    if !output.status.success() {
        return Err(command_failed(CONVERTER, &output));
    }
    Ok(CONVERTER)
}

/// 查找当前平台可调用的 LibreOffice 可执行文件。
pub(crate) fn find_libreoffice() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let standard = vec![PathBuf::from(MACOS_SOFFICE)];
    #[cfg(target_os = "linux")]
    let standard = Vec::new();
    #[cfg(target_os = "windows")]
    let standard = windows_libreoffice_candidates();

    find_libreoffice_in(&standard, std::env::var_os("PATH").as_deref())
}

/// 按标准路径、soffice、libreoffice 的顺序选择可执行文件。
fn find_libreoffice_in(standard: &[PathBuf], path_value: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(path) = standard.iter().find(|path| is_executable(path)) {
        return Some(path.clone());
    }

    let path_value = path_value?;
    for directory in std::env::split_paths(path_value) {
        for name in path_executable_names() {
            let candidate = directory.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// 返回当前平台在 PATH 中查找的 LibreOffice 可执行文件名。
#[cfg(unix)]
fn path_executable_names() -> [&'static str; 2] {
    ["soffice", "libreoffice"]
}

/// 返回 Windows 在 PATH 中查找的 LibreOffice 可执行文件名。
#[cfg(target_os = "windows")]
fn path_executable_names() -> [&'static str; 2] {
    ["soffice.exe", "libreoffice.exe"]
}

/// 判断候选路径是否是当前用户可执行的普通文件。
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// 判断 Windows 候选路径是否是普通文件。
#[cfg(target_os = "windows")]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// 返回 Windows 标准目录和 App Paths 中的 LibreOffice 候选。
#[cfg(target_os = "windows")]
fn windows_libreoffice_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(directory) = std::env::var_os(variable) {
            candidates.push(
                PathBuf::from(directory)
                    .join("LibreOffice")
                    .join("program")
                    .join("soffice.exe"),
            );
        }
    }
    for root in ["HKCU", "HKLM"] {
        for view in ["/reg:64", "/reg:32"] {
            if let Some(path) = query_windows_app_path(root, view) {
                candidates.push(path);
            }
        }
    }
    candidates
}

/// 从 Windows App Paths 被动读取 LibreOffice 可执行文件。
#[cfg(target_os = "windows")]
fn query_windows_app_path(root: &str, view: &str) -> Option<PathBuf> {
    let mut command = StdCommand::new("reg.exe");
    let key = format!(r"{root}\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\soffice.exe");
    command
        .creation_flags(CREATE_NO_WINDOW)
        .arg("query")
        .arg(key)
        .args(["/ve", view]);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_windows_app_path(&String::from_utf8_lossy(&output.stdout))
}

/// 解析 `reg query` 返回的 App Paths 默认值。
#[cfg(target_os = "windows")]
fn parse_windows_app_path(output: &str) -> Option<PathBuf> {
    output.lines().find_map(|line| {
        ["REG_SZ", "REG_EXPAND_SZ"].iter().find_map(|kind| {
            line.split_once(kind)
                .map(|(_, value)| PathBuf::from(value.trim().trim_matches('"')))
                .filter(|path| !path.as_os_str().is_empty())
        })
    })
}

/// 写入不信任任何文档路径的最高宏安全配置。
async fn write_macro_security_profile(profile_dir: &Path) -> Result<(), OfficeConvertError> {
    let user_dir = profile_dir.join("user");
    tokio::fs::create_dir_all(&user_dir).await?;
    tokio::fs::write(
        user_dir.join("registrymodifications.xcu"),
        MACRO_SECURITY_CONFIG,
    )
    .await?;
    Ok(())
}

/// 构造使用隔离用户配置的 LibreOffice headless 命令。
fn build_command(
    executable: &Path,
    profile_url: &str,
    input_path: &Path,
    output_dir: &Path,
) -> Command {
    let mut command = Command::new(executable);
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    command.args([
        "--headless",
        "--nologo",
        "--nodefault",
        "--nolockcheck",
        "--norestore",
    ]);
    command.arg(format!("-env:UserInstallation={profile_url}"));
    command.args(["--convert-to", "pdf", "--outdir"]);
    command.arg(output_dir);
    command.arg(input_path);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    #[test]
    fn prefers_macos_standard_path_before_path_entries() {
        let root = test_root("mac-discovery");
        let standard = root.join("Applications/LibreOffice.app/Contents/MacOS/soffice");
        let path_dir = root.join("bin");
        create_file(&standard);
        create_file(&path_dir.join("soffice"));
        let path = std::env::join_paths([path_dir]).unwrap();

        assert_eq!(
            find_libreoffice_in(std::slice::from_ref(&standard), Some(&path)),
            Some(standard.clone())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finds_soffice_before_libreoffice_in_path() {
        let root = test_root("linux-discovery");
        let path_dir = root.join("bin");
        let [preferred, fallback] = path_executable_names();
        create_file(&path_dir.join(preferred));
        create_file(&path_dir.join(fallback));
        let path = std::env::join_paths([path_dir.clone()]).unwrap();

        assert_eq!(
            find_libreoffice_in(&[], Some(&path)),
            Some(path_dir.join(preferred))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn returns_none_when_no_candidates_exist() {
        let root = test_root("missing-discovery");
        let path = std::env::join_paths([root.join("empty-bin")]).unwrap();

        assert_eq!(find_libreoffice_in(&[], Some(&path)), None);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn writes_very_high_macro_security_without_trusted_locations() {
        let root = test_root("macro-security");
        write_macro_security_profile(&root).await.unwrap();
        let contents = fs::read_to_string(root.join("user/registrymodifications.xcu")).unwrap();

        assert!(contents.contains("MacroSecurityLevel"));
        assert!(contents.contains("<value>3</value>"));
        assert!(!contents.contains("SecureURL"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn builds_isolated_headless_conversion_command() {
        let executable = Path::new("/opt/libreoffice/program/soffice");
        let profile_url = "file:///tmp/printbridge-profile";
        let input = Path::new("/tmp/job.docx");
        let output_dir = Path::new("/tmp");
        let command = build_command(executable, profile_url, input, output_dir);
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert!(args.contains(&"--headless".to_string()));
        assert!(args.contains(&"--nologo".to_string()));
        assert!(args.contains(&"--nodefault".to_string()));
        assert!(args.contains(&"--nolockcheck".to_string()));
        assert!(args.contains(&"--norestore".to_string()));
        assert!(args.contains(&"--convert-to".to_string()));
        assert!(args.contains(&"pdf".to_string()));
        assert!(args.contains(&format!("-env:UserInstallation={profile_url}")));
        assert!(args.contains(&input.display().to_string()));
        assert!(args.contains(&output_dir.display().to_string()));
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "print-bridge-libreoffice-{name}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn create_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }
    }
}
