use crate::{
    config::AgentConfig,
    logs::LogStore,
    logs::TaskLogEntry,
    printing::{default_backend, PrintBackend},
    protocol::validate_origin,
    queue::QueueState,
    remote_store::RemoteStore,
};
use std::{io, path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, Mutex, Notify, RwLock};

const STATUS_EVENT_CAPACITY: usize = 128;

/// Tauri 命令、HTTP 路由和 worker 共享的运行时状态。
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AgentConfig>>,
    pub config_path: Option<PathBuf>,
    pub logs: Arc<Mutex<LogStore>>,
    pub status_events: broadcast::Sender<TaskLogEntry>,
    pub queue: Arc<Mutex<QueueState>>,
    pub queue_notify: Arc<Notify>,
    pub remote_notify: Arc<Notify>,
    pub print_lock: Arc<Mutex<()>>,
    pub printing: Arc<dyn PrintBackend + Send + Sync>,
    pub remote_store: Option<Arc<RemoteStore>>,
}

impl AppState {
    /// 使用当前平台默认打印后端创建应用状态。
    pub fn new(config: AgentConfig) -> Self {
        Self::with_printing(config, default_backend())
    }

    /// 创建会把配置持久化到指定路径的应用状态。
    pub fn with_config_path(config: AgentConfig, config_path: PathBuf) -> Self {
        Self::with_printing_and_config_path(config, default_backend(), Some(config_path))
    }

    /// 使用注入的打印后端创建可持久化的应用状态。
    pub fn with_config_path_and_printing(
        config: AgentConfig,
        config_path: PathBuf,
        printing: Box<dyn PrintBackend + Send + Sync>,
    ) -> Self {
        Self::with_printing_and_config_path(config, printing, Some(config_path))
    }

    /// 使用注入的打印后端创建不绑定配置文件的应用状态。
    pub fn with_printing(
        config: AgentConfig,
        printing: Box<dyn PrintBackend + Send + Sync>,
    ) -> Self {
        Self::with_printing_and_config_path(config, printing, None)
    }

    /// 构造所有公开构造函数共用的状态容器。
    fn with_printing_and_config_path(
        config: AgentConfig,
        printing: Box<dyn PrintBackend + Send + Sync>,
        config_path: Option<PathBuf>,
    ) -> Self {
        let (status_events, _) = broadcast::channel(STATUS_EVENT_CAPACITY);
        Self {
            config: Arc::new(RwLock::new(config.normalized())),
            config_path,
            logs: Arc::new(Mutex::new(LogStore::default())),
            status_events,
            queue: Arc::new(Mutex::new(QueueState::default())),
            queue_notify: Arc::new(Notify::new()),
            remote_notify: Arc::new(Notify::new()),
            print_lock: Arc::new(Mutex::new(())),
            printing: Arc::from(printing),
            remote_store: None,
        }
    }

    /// 注入远程任务 SQLite 存储，供生产启动和相关测试使用。
    pub fn with_remote_store(mut self, remote_store: RemoteStore) -> Self {
        self.remote_store = Some(Arc::new(remote_store));
        self
    }

    /// 校验、按需持久化并应用新配置。
    pub async fn save_config(&self, config: AgentConfig) -> Result<AgentConfig, io::Error> {
        let config = config.normalized();
        for origin in &config.security.allowed_origins {
            validate_origin(origin)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        }

        if let Some(path) = &self.config_path {
            config.save(path)?;
        }

        *self.config.write().await = config.clone();
        self.remote_notify.notify_waiters();
        Ok(config)
    }

    /// 为单个 WebSocket 连接订阅 worker 状态事件。
    pub fn subscribe_status_events(&self) -> broadcast::Receiver<TaskLogEntry> {
        self.status_events.subscribe()
    }

    /// 向活跃订阅者广播状态事件。
    pub fn broadcast_status(&self, entry: TaskLogEntry) {
        let _ = self.status_events.send(entry);
    }
}
