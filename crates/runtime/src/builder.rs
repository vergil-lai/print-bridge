use crate::{
    config::AgentConfig,
    html::{browser::BrowserHtmlRenderer, resource_policy::ResourcePolicy, HtmlRenderer},
    printing::{self, PrintBackend},
    remote_store::RemoteStore,
    state::AgentState,
    task_history::TaskHistoryStore,
    AgentRuntime, RuntimeError,
};
use std::{path::PathBuf, sync::Arc};

/// Agent 配置、持久化数据和进程内运行文件的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl RuntimePaths {
    /// 使用配置文件与数据目录构造路径集合。
    pub fn new(config_path: PathBuf, data_dir: PathBuf, runtime_dir: PathBuf) -> Self {
        Self {
            config_path,
            data_dir,
            runtime_dir,
        }
    }

    /// 返回远程任务数据库路径。
    pub fn remote_store_path(&self) -> PathBuf {
        self.data_dir.join("remote.sqlite3")
    }

    /// 返回任务历史数据库路径。
    pub fn task_history_path(&self) -> PathBuf {
        self.data_dir.join("task_history.sqlite3")
    }
}

/// 组装 Agent runtime 所需平台实现和持久化路径。
pub struct RuntimeBuilder {
    paths: RuntimePaths,
    printing: Option<Box<dyn PrintBackend + Send + Sync>>,
    html_renderer: Option<Arc<dyn HtmlRenderer>>,
}

impl RuntimeBuilder {
    /// 使用明确的配置、数据和运行目录开始组装。
    pub fn new(paths: RuntimePaths) -> Self {
        Self {
            paths,
            printing: None,
            html_renderer: None,
        }
    }

    /// 注入当前产品选择的平台打印后端。
    pub fn print_backend(mut self, printing: Box<dyn PrintBackend + Send + Sync>) -> Self {
        self.printing = Some(printing);
        self
    }

    /// 注入 HTML renderer，供测试和平台产品替换默认实现。
    pub fn html_renderer(mut self, renderer: Arc<dyn HtmlRenderer>) -> Self {
        self.html_renderer = Some(renderer);
        self
    }

    /// 创建目录、加载配置和打开持久化 stores。
    pub fn build(self) -> Result<AgentRuntime, RuntimeError> {
        std::fs::create_dir_all(&self.paths.data_dir)?;
        std::fs::create_dir_all(&self.paths.runtime_dir)?;
        if let Some(parent) = self.paths.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let config = AgentConfig::load(&self.paths.config_path)?;
        let remote_store = RemoteStore::open(&self.paths.remote_store_path())?;
        let task_history = TaskHistoryStore::open(&self.paths.task_history_path())?;
        let printing = self.printing.unwrap_or_else(printing::default_backend);
        let html_renderer = self.html_renderer.unwrap_or_else(|| {
            Arc::new(BrowserHtmlRenderer::new(ResourcePolicy::system())) as Arc<dyn HtmlRenderer>
        });
        let state = AgentState::with_config_path_printing_and_html_renderer(
            config,
            self.paths.config_path.clone(),
            printing,
            html_renderer,
        )
        .with_remote_store(remote_store)
        .with_task_history_store(task_history);

        Ok(AgentRuntime::new(self.paths, state))
    }
}
