#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use print_bridge_cli::{client::LocalCommandClient, Command, CommandResult};
use print_bridge_core::{
    config::AgentConfig,
    printing::{
        PaperInfo, PrintBackend, PrintOptions, PrintResult, PrintSubmission, PrinterInfo,
        RawPrintOptions,
    },
};
use print_bridge_runtime::{ipc, RuntimeBuilder, RuntimePaths};

struct NoopPrintBackend;

impl PrintBackend for NoopPrintBackend {
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
        Ok(Vec::new())
    }
    fn list_papers(&self, _: &str) -> PrintResult<Vec<PaperInfo>> {
        Ok(Vec::new())
    }
    fn print_pdf(&self, _: &std::path::Path, _: &PrintOptions) -> PrintResult<PrintSubmission> {
        unreachable!()
    }
    fn print_raw(&self, _: &[u8], _: &RawPrintOptions) -> PrintResult<PrintSubmission> {
        unreachable!()
    }
}

#[tokio::test]
async fn unix_ipc_roundtrips_uses_0660_and_cleans_up() {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::new(
        temp.path().join("config.json"),
        temp.path().join("data"),
        temp.path().join("run"),
    );
    let mut config = AgentConfig::default();
    config.service.port = 0;
    config.save(&paths.config_path).unwrap();
    let runtime = RuntimeBuilder::new(paths.clone())
        .print_backend(Box::new(NoopPrintBackend))
        .build()
        .unwrap();
    let handle = runtime.start().await.unwrap();
    let socket = ipc::socket_path(&paths.runtime_dir);

    let result = LocalCommandClient::execute(&socket, Command::Status)
        .await
        .unwrap();
    assert!(matches!(result, CommandResult::Status(status) if status.running));
    assert_eq!(
        std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o660
    );

    handle.shutdown().await.unwrap();
    assert!(!socket.exists());
}
