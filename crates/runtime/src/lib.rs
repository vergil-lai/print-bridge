//! PrintBridge Agent 的运行状态、平台适配和后台任务。

pub use print_bridge_core::{config, ip_whitelist, protocol, remote_protocol};

pub mod agent;
pub mod agent_guard;
pub mod builder;
pub mod command_executor;
pub mod document;
pub mod download;
pub mod html;
pub mod ipc;
pub mod logs;
pub mod office;
pub mod printing;
pub mod queue;
pub mod remote_client;
pub mod remote_store;
pub mod remote_worker;
pub mod server;
pub mod state;
pub mod task_history;
pub mod test_print;

pub use agent::{AgentHandle, AgentRuntime, RuntimeError};
pub use builder::{RuntimeBuilder, RuntimePaths};
pub use command_executor::RuntimeCommandExecutor;
