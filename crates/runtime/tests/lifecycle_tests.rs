use print_bridge_core::{
    config::AgentConfig,
    printing::{
        PaperInfo, PrintBackend, PrintOptions, PrintResult, PrintSubmission, PrinterInfo,
        RawPrintOptions,
    },
};
use print_bridge_runtime::{RuntimeBuilder, RuntimePaths};

struct NoopPrintBackend;

impl PrintBackend for NoopPrintBackend {
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
        Ok(Vec::new())
    }

    fn list_papers(&self, _printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
        Ok(Vec::new())
    }

    fn print_pdf(
        &self,
        _path: &std::path::Path,
        _options: &PrintOptions,
    ) -> PrintResult<PrintSubmission> {
        unreachable!("lifecycle test does not print")
    }

    fn print_raw(&self, _data: &[u8], _options: &RawPrintOptions) -> PrintResult<PrintSubmission> {
        unreachable!("lifecycle test does not print")
    }
}

#[tokio::test]
async fn shutdown_stops_listener_and_workers() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.json");
    let data_dir = temp.path().join("data");
    let runtime_dir = temp.path().join("run");
    let mut config = AgentConfig::default();
    config.service.port = 0;
    config.save(&config_path).unwrap();

    let runtime = RuntimeBuilder::new(RuntimePaths::new(
        config_path,
        data_dir.clone(),
        runtime_dir.clone(),
    ))
    .print_backend(Box::new(NoopPrintBackend))
    .build()
    .unwrap();
    let handle = runtime.start().await.unwrap();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], handle.listen_addr().port()));

    assert!(tokio::net::TcpStream::connect(addr).await.is_ok());
    handle.shutdown().await.unwrap();
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    assert!(data_dir.join("remote.sqlite3").exists());
    assert!(data_dir.join("task_history.sqlite3").exists());
    assert!(runtime_dir.exists());
}
