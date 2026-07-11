use std::path::PathBuf;

use print_bridge_runtime::RuntimePaths;

/// 返回 headless 系统服务的固定配置、状态和运行目录。
pub fn system_paths() -> RuntimePaths {
    RuntimePaths::new(
        env_path("PRINT_BRIDGE_CONFIG_PATH", "/etc/print-bridge/config.json"),
        env_path("PRINT_BRIDGE_DATA_DIR", "/var/lib/print-bridge"),
        env_path("PRINT_BRIDGE_RUNTIME_DIR", "/run/print-bridge"),
    )
}

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}
