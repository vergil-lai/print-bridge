use crate::protocol::EffectivePaper;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

pub const DEFAULT_PORT: u16 = 17890;

/// 本地 PrintBridge Agent 的持久化配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentConfig {
    pub service: ServiceConfig,
    pub security: SecurityConfig,
    pub printing: PrintingConfig,
    pub limits: LimitsConfig,
    pub app: AppConfig,
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
            app: AppConfig { autostart: false },
        }
    }
}

impl AgentConfig {
    /// 保持兼容字段为本机默认值；服务端实际监听地址由 server 模块决定。
    pub fn normalized(mut self) -> Self {
        self.service.host = "127.0.0.1".to_string();
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
