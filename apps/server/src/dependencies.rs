use std::{
    env,
    path::{Path, PathBuf},
};

/// 验证 headless 系统用户运行打印和文档转换所需的外部程序。
pub fn preflight() -> Result<(), String> {
    for program in ["lp", "lpstat", "lpoptions"] {
        require_any(&[program], program, "install cups-client")?;
    }
    require_any(
        &["soffice", "libreoffice"],
        "LibreOffice",
        "install libreoffice",
    )?;
    require_any(
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ],
        "Chrome/Chromium",
        "install Google Chrome or Chromium system-wide",
    )?;
    Ok(())
}

fn require_any(candidates: &[&str], label: &str, suggestion: &str) -> Result<PathBuf, String> {
    candidates
        .iter()
        .find_map(|candidate| find_on_path(candidate))
        .ok_or_else(|| format!("missing {label}; {suggestion}"))
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let direct = Path::new(program);
    if direct.is_absolute() && direct.is_file() {
        return Some(direct.to_path_buf());
    }
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}
