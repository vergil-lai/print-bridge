fn normalized(entry: &str) -> String {
    entry
        .trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

fn contains_path_entry(value: &str, entry: &str) -> bool {
    let expected = normalized(entry);
    value
        .split(';')
        .any(|candidate| !candidate.trim().is_empty() && normalized(candidate) == expected)
}

fn add_path_entry(value: &str, entry: &str) -> String {
    if contains_path_entry(value, entry) {
        return value.to_owned();
    }
    let value = value.trim_end_matches(';');
    if value.is_empty() {
        entry.to_owned()
    } else {
        format!("{value};{entry}")
    }
}

fn remove_path_entry(value: &str, entry: &str) -> String {
    let expected = normalized(entry);
    value
        .split(';')
        .filter(|candidate| !candidate.trim().is_empty() && normalized(candidate) != expected)
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{add_path_entry, contains_path_entry, remove_path_entry};
    use crate::cli_integration::{CliIntegrationStatus, CliIntegrationStatusKind};
    use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::PathBuf, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
        },
        UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        },
    };

    const USER_ENVIRONMENT: &str = "Environment";
    const MACHINE_ENVIRONMENT: &str =
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

    /// 将 Win32 错误码转换为可显示的系统错误信息。
    fn os_error(code: u32) -> String {
        io::Error::from_raw_os_error(code as i32).to_string()
    }

    pub(crate) fn status() -> Result<CliIntegrationStatus, String> {
        let directory = install_directory()?;
        let cli = directory.join("print-bridge.exe");
        if !cli.is_file() {
            return Ok(CliIntegrationStatus {
                kind: CliIntegrationStatusKind::Unavailable,
                command_path: Some(cli.display().to_string()),
                path_ready: false,
            });
        }
        let entry = directory.display().to_string();
        let (user_path, _) = read_path(HKEY_CURRENT_USER, USER_ENVIRONMENT, true)?;
        let (machine_path, _) = read_path(HKEY_LOCAL_MACHINE, MACHINE_ENVIRONMENT, false)?;
        let kind = if contains_path_entry(&user_path, &entry) {
            CliIntegrationStatusKind::Installed
        } else if contains_path_entry(&machine_path, &entry) {
            CliIntegrationStatusKind::InstalledSystem
        } else {
            CliIntegrationStatusKind::NotInstalled
        };
        Ok(CliIntegrationStatus {
            kind,
            command_path: Some(cli.display().to_string()),
            path_ready: contains_path_entry(&user_path, &entry)
                || contains_path_entry(&machine_path, &entry),
        })
    }

    pub(crate) fn install() -> Result<CliIntegrationStatus, String> {
        let directory = install_directory()?;
        if !directory.join("print-bridge.exe").is_file() {
            return status();
        }
        let entry = directory.display().to_string();
        let (value, kind) = read_path(HKEY_CURRENT_USER, USER_ENVIRONMENT, true)?;
        write_user_path(&add_path_entry(&value, &entry), kind)?;
        broadcast_environment_change();
        status()
    }

    pub(crate) fn uninstall() -> Result<CliIntegrationStatus, String> {
        let directory = install_directory()?;
        let entry = directory.display().to_string();
        let (value, kind) = read_path(HKEY_CURRENT_USER, USER_ENVIRONMENT, true)?;
        write_user_path(&remove_path_entry(&value, &entry), kind)?;
        broadcast_environment_change();
        status()
    }

    fn install_directory() -> Result<PathBuf, String> {
        std::env::current_exe()
            .map_err(|error| error.to_string())?
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| "PrintBridge executable has no parent directory".to_owned())
    }

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    fn read_path(root: HKEY, subkey: &str, writable: bool) -> Result<(String, u32), String> {
        unsafe {
            let mut key = ptr::null_mut();
            let access = KEY_READ | if writable { KEY_SET_VALUE } else { 0 };
            let code = RegOpenKeyExW(root, wide(subkey).as_ptr(), 0, access, &mut key);
            if code != ERROR_SUCCESS {
                return Err(os_error(code));
            }
            let name = wide("Path");
            let mut kind = REG_EXPAND_SZ;
            let mut length = 0;
            let code = RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                ptr::null_mut(),
                &mut length,
            );
            if code == ERROR_FILE_NOT_FOUND {
                RegCloseKey(key);
                return Ok((String::new(), REG_EXPAND_SZ));
            }
            if code != ERROR_SUCCESS {
                RegCloseKey(key);
                return Err(os_error(code));
            }
            let mut buffer = vec![0u16; length as usize / 2];
            let code = RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut length,
            );
            RegCloseKey(key);
            if code != ERROR_SUCCESS {
                return Err(os_error(code));
            }
            while buffer.last() == Some(&0) {
                buffer.pop();
            }
            Ok((String::from_utf16_lossy(&buffer), kind))
        }
    }

    fn write_user_path(value: &str, kind: u32) -> Result<(), String> {
        unsafe {
            let mut key = ptr::null_mut();
            let code = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(USER_ENVIRONMENT).as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            );
            if code != ERROR_SUCCESS {
                return Err(os_error(code));
            }
            let encoded = wide(value);
            let value_kind = if kind == REG_SZ {
                REG_SZ
            } else {
                REG_EXPAND_SZ
            };
            let code = RegSetValueExW(
                key,
                wide("Path").as_ptr(),
                0,
                value_kind,
                encoded.as_ptr().cast(),
                (encoded.len() * 2) as u32,
            );
            RegCloseKey(key);
            if code == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(os_error(code))
            }
        }
    }

    fn broadcast_environment_change() {
        let environment = wide("Environment");
        let mut result = 0;
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5_000,
                &mut result,
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use platform::{install, status, uninstall};

#[cfg(test)]
mod tests {
    use super::{add_path_entry, remove_path_entry};

    #[test]
    fn add_path_entry_deduplicates_case_and_trailing_separator() {
        let value = r"C:\Windows;C:\Program Files\PrintBridge";
        assert_eq!(
            add_path_entry(value, r"c:\program files\printbridge\"),
            value
        );
    }

    #[test]
    fn remove_path_entry_preserves_unrelated_entries() {
        let value = r"C:\Windows;C:\Program Files\PrintBridge;C:\Tools";
        assert_eq!(
            remove_path_entry(value, r"c:\program files\printbridge\"),
            r"C:\Windows;C:\Tools"
        );
    }
}
