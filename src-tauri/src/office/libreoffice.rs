use super::{
    command_failed, execute_converter_command, OfficeConvertError, OFFICE_CONVERSION_TIMEOUT,
};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tokio::process::Command;
use url::Url;

const CONVERTER: &str = "LibreOffice";
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
fn find_libreoffice() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let standard = Some(Path::new(MACOS_SOFFICE));
    #[cfg(target_os = "linux")]
    let standard = None;

    find_libreoffice_in(standard, std::env::var_os("PATH").as_deref())
}

/// 按标准路径、soffice、libreoffice 的顺序选择可执行文件。
fn find_libreoffice_in(standard: Option<&Path>, path_value: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(path) = standard.filter(|path| is_executable(path)) {
        return Some(path.to_path_buf());
    }

    let path_value = path_value?;
    for directory in std::env::split_paths(path_value) {
        for name in ["soffice", "libreoffice"] {
            let candidate = directory.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// 判断候选路径是否是当前用户可执行的普通文件。
fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
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
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
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
            find_libreoffice_in(Some(&standard), Some(&path)),
            Some(standard.clone())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finds_soffice_before_libreoffice_in_path() {
        let root = test_root("linux-discovery");
        let path_dir = root.join("bin");
        create_file(&path_dir.join("soffice"));
        create_file(&path_dir.join("libreoffice"));
        let path = std::env::join_paths([path_dir.clone()]).unwrap();

        assert_eq!(
            find_libreoffice_in(None, Some(&path)),
            Some(path_dir.join("soffice"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn returns_none_when_no_candidates_exist() {
        let root = test_root("missing-discovery");
        let path = std::env::join_paths([root.join("empty-bin")]).unwrap();

        assert_eq!(find_libreoffice_in(None, Some(&path)), None);
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
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}
