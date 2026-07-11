use print_bridge_cli::{Command, CommandExecutor, CommandResult};
use print_bridge_core::{
    config::AgentConfig,
    printing::{
        PaperInfo, PrintBackend, PrintOptions, PrintResult, PrintSubmission, PrinterInfo,
        RawPrintOptions,
    },
};
use print_bridge_runtime::{RuntimeBuilder, RuntimeCommandExecutor, RuntimePaths};

struct FixturePrintBackend;

impl PrintBackend for FixturePrintBackend {
    fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
        Ok(vec![PrinterInfo::new("Office".into(), true)])
    }

    fn list_papers(&self, printer_name: &str) -> PrintResult<Vec<PaperInfo>> {
        assert_eq!(printer_name, "Office");
        Ok(vec![PaperInfo {
            id: "a4".into(),
            name: "A4".into(),
            width_mm: 210.0,
            height_mm: 297.0,
        }])
    }

    fn print_pdf(
        &self,
        _path: &std::path::Path,
        _options: &PrintOptions,
    ) -> PrintResult<PrintSubmission> {
        unreachable!()
    }

    fn print_raw(&self, _data: &[u8], _options: &RawPrintOptions) -> PrintResult<PrintSubmission> {
        unreachable!()
    }
}

#[tokio::test]
async fn runtime_executor_serves_config_printers_and_papers() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.json");
    AgentConfig::default().save(&config_path).unwrap();
    let runtime = RuntimeBuilder::new(RuntimePaths::new(
        config_path,
        temp.path().join("data"),
        temp.path().join("run"),
    ))
    .print_backend(Box::new(FixturePrintBackend))
    .build()
    .unwrap();
    let handle = runtime.start().await.unwrap();
    let executor = RuntimeCommandExecutor::new(handle.state(), handle.listen_addr());

    assert!(matches!(
        executor.execute(Command::GetConfig).await.unwrap(),
        CommandResult::Config(_)
    ));
    assert!(matches!(
        executor.execute(Command::ListPrinters).await.unwrap(),
        CommandResult::Printers(printers) if printers[0].name == "Office"
    ));
    assert!(matches!(
        executor
            .execute(Command::ListPapers { printer_name: "Office".into() })
            .await
            .unwrap(),
        CommandResult::Papers(papers) if papers[0].id == "a4"
    ));

    handle.shutdown().await.unwrap();
}
