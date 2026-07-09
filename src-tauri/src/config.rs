use crate::{
    ip_whitelist::{normalize_allowed_ips, REQUIRED_LOOPBACK_IP},
    protocol::EffectivePaper,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

/// 系统配置目录中使用的应用目录名。
pub const APP_CONFIG_DIR_NAME: &str = "com.vergil.printbridge";
/// Agent 配置文件名。
pub const CONFIG_FILE_NAME: &str = "config.json";
/// 本地任务历史数据库文件名。
pub const TASK_HISTORY_FILE_NAME: &str = "task_history.sqlite3";
/// 覆盖配置文件路径的环境变量名。
pub const CONFIG_PATH_OVERRIDE_ENV: &str = "PRINT_BRIDGE_CONFIG_PATH";
/// 覆盖数据目录路径的环境变量名。
pub const DATA_DIR_OVERRIDE_ENV: &str = "PRINT_BRIDGE_DATA_DIR";

/// 用户可配置的最小本地服务端口。
pub const MIN_SERVICE_PORT: u16 = 10000;
/// 用户可配置的最大本地服务端口。
pub const MAX_SERVICE_PORT: u16 = u16::MAX;
/// PrintBridge Agent 默认本地服务端口。
pub const DEFAULT_PORT: u16 = 17890;
/// 远程任务轮询允许的最小间隔秒数。
pub const MIN_REMOTE_POLL_INTERVAL_SECONDS: u64 = 3;
/// 远程状态回报允许的最小重试次数。
pub const MIN_REMOTE_MAX_REPORT_RETRIES: u32 = 1;

/// 返回默认 IP 白名单，保留本机回环地址。
fn default_allowed_ips() -> Vec<String> {
    vec![REQUIRED_LOOPBACK_IP.to_string()]
}

fn default_ui_language() -> UiLanguage {
    UiLanguage::ZhCn
}

/// 返回 CLI 使用的配置文件路径。
pub fn cli_config_path() -> Result<PathBuf, io::Error> {
    if let Some(path) = env::var_os(CONFIG_PATH_OVERRIDE_ENV) {
        return Ok(PathBuf::from(path));
    }

    Ok(cli_config_dir()?.join(CONFIG_FILE_NAME))
}

/// 返回 CLI 使用的任务历史数据库路径。
pub fn cli_task_history_path() -> Result<PathBuf, io::Error> {
    if let Some(dir) = env::var_os(DATA_DIR_OVERRIDE_ENV) {
        return Ok(PathBuf::from(dir).join(TASK_HISTORY_FILE_NAME));
    }

    Ok(cli_config_dir()?.join(TASK_HISTORY_FILE_NAME))
}

fn cli_config_dir() -> Result<PathBuf, io::Error> {
    if let Some(dir) = env::var_os(DATA_DIR_OVERRIDE_ENV) {
        return Ok(PathBuf::from(dir));
    }

    platform_config_dir().map(|dir| dir.join(APP_CONFIG_DIR_NAME))
}

#[cfg(target_os = "windows")]
fn platform_config_dir() -> Result<PathBuf, io::Error> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set"))
}

#[cfg(target_os = "macos")]
fn platform_config_dir() -> Result<PathBuf, io::Error> {
    home_dir().map(|home| home.join("Library").join("Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_config_dir() -> Result<PathBuf, io::Error> {
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir));
    }

    home_dir().map(|home| home.join(".config"))
}

fn home_dir() -> Result<PathBuf, io::Error> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
}

/// 本地 PrintBridge Agent 的持久化配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentConfig {
    pub service: ServiceConfig,
    pub security: SecurityConfig,
    pub printing: PrintingConfig,
    pub limits: LimitsConfig,
    pub app: AppConfig,
    #[serde(default)]
    pub remote: RemoteConfig,
}

/// 本地 HTTP/WebSocket 服务绑定设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub host: String,
    pub port: u16,
}

/// 允许打开 PrintBridge WebSocket 会话的浏览器 Origin。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_allowed_ips")]
    pub allowed_ips: Vec<String>,
}

/// 打印任务未提供覆盖项时使用的默认打印设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintingConfig {
    pub default_printer: Option<String>,
    pub default_paper: Option<EffectivePaper>,
    pub default_copies: u16,
}

/// 下载、批量任务和打印份数的运行时安全限制。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsConfig {
    pub max_file_size_mb: u64,
    pub max_batch_jobs: usize,
    pub max_copies: u16,
    pub download_timeout_seconds: u64,
}

/// 桌面应用偏好设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub autostart: bool,
    #[serde(default = "default_ui_language")]
    pub language: UiLanguage,
}

/// 桌面 UI 语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiLanguage {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
}

/// 远程任务轮询和状态回报设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub enabled: bool,
    pub endpoint_url: Option<String>,
    pub bearer_token: Option<String>,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub poll_interval_seconds: u64,
    pub max_report_retries: u32,
    pub history_retention_days: u32,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint_url: None,
            bearer_token: None,
            device_id: None,
            device_name: None,
            poll_interval_seconds: 10,
            max_report_retries: 10,
            history_retention_days: 3,
        }
    }
}

impl Default for AgentConfig {
    /// 创建首次运行配置，并使用仅限本机访问的服务默认值。
    fn default() -> Self {
        Self {
            service: ServiceConfig {
                host: "127.0.0.1".to_string(),
                port: DEFAULT_PORT,
            },
            security: SecurityConfig {
                allowed_origins: Vec::new(),
                allowed_ips: default_allowed_ips(),
            },
            printing: PrintingConfig {
                default_printer: None,
                default_paper: None,
                default_copies: 1,
            },
            limits: LimitsConfig {
                max_file_size_mb: 20,
                max_batch_jobs: 20,
                max_copies: 100,
                download_timeout_seconds: 30,
            },
            app: AppConfig {
                autostart: false,
                language: UiLanguage::ZhCn,
            },
            remote: RemoteConfig::default(),
        }
    }
}

impl AgentConfig {
    /// 保持兼容字段为本机默认值；服务端实际监听地址由 server 模块决定。
    pub fn normalized(mut self) -> Self {
        self.service.host = "127.0.0.1".to_string();
        self.security.allowed_ips = normalize_allowed_ips(self.security.allowed_ips);
        self
    }

    /// 从磁盘加载配置；文件不存在时返回默认配置。
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)?;
        serde_json::from_str::<Self>(&content)
            .map(Self::normalized)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// 把规范化后的配置保存到磁盘，必要时创建父目录。
    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let config = self.clone().normalized();
        let content =
            serde_json::to_string_pretty(&config).expect("AgentConfig should always serialize");
        fs::write(path, content)
    }
}
