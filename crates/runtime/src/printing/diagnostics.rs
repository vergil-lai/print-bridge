use super::PrintBackend;
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::VecDeque, path::Path, sync::Mutex};

const CAPACITY: usize = 2000;
static EVENTS: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());

/// 保留最近的结构化打印诊断；不记录文档地址、内容或认证配置。
#[cfg(any(test, target_os = "windows"))]
pub(super) fn record(operation: &str, printer: &str, stage: &str, details: Value) {
    let entry = json!({
        "time": super::submitted_at_rfc3339(),
        "operation": operation,
        "printer": printer,
        "stage": stage,
        "details": details,
    });
    log::info!(target: "print_bridge::paper_diagnostics", "{entry}");
    push(
        &mut EVENTS.lock().unwrap_or_else(|error| error.into_inner()),
        entry,
    );
}

/// 按容量丢弃最早的诊断事件。
#[cfg(any(test, target_os = "windows"))]
fn push(events: &mut VecDeque<Value>, entry: Value) {
    events.push_back(entry);
    while events.len() > CAPACITY {
        events.pop_front();
    }
}

/// 返回当前进程中保留的诊断快照。
fn snapshot() -> Vec<Value> {
    EVENTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .cloned()
        .collect()
}

/// 把平台探测结果转换为可序列化的成功或失败记录。
fn outcome<T: Serialize, E: std::fmt::Display>(result: Result<T, E>) -> Value {
    match result {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(error) => json!({ "ok": false, "error": error.to_string() }),
    }
}

/// 收集近期事件并重新只读探测纸型，单台打印机失败不阻止报告生成。
pub fn collect_report(backend: &dyn PrintBackend) -> Value {
    // 先保存现场，避免重新探测的事件淘汰导致失败的原始记录。
    let recent_events = snapshot();
    let printers = backend.list_printers();
    let papers = printers.as_ref().map(|printers| {
        printers.iter().map(|printer| {
            json!({ "printer": printer.name, "papers": outcome(backend.list_papers(&printer.name)) })
        }).collect::<Vec<_>>()
    }).unwrap_or_default();
    #[cfg(target_os = "windows")]
    let environment = outcome(super::windows::diagnostic_environment());
    #[cfg(not(target_os = "windows"))]
    let environment = json!({});
    json!({
        "schema_version": 1,
        "app_version": env!("CARGO_PKG_VERSION"),
        "generated_at": super::submitted_at_rfc3339(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "environment": environment,
        "printers": outcome(printers),
        "paper_probes": papers,
        "recent_events": recent_events,
        "events_after_probe": snapshot(),
        "event_capacity": CAPACITY,
        "retention": "Recent events from the current process only; oldest events are discarded at capacity.",
    })
}

/// 将诊断报告写入用户选择的 JSON 文件。
pub fn export_report(path: &Path, backend: &dyn PrintBackend) -> std::io::Result<()> {
    let report = collect_report(backend);
    std::fs::write(path, serde_json::to_vec_pretty(&report)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printing::{
        PaperInfo, PrintError, PrintOptions, PrintResult, PrintSubmission, PrinterInfo,
    };

    struct FailingBackend {
        discovery_failure: bool,
    }
    impl PrintBackend for FailingBackend {
        fn list_printers(&self) -> PrintResult<Vec<PrinterInfo>> {
            if self.discovery_failure {
                Err(PrintError::PrinterNotFound("test printer".into()))
            } else {
                Ok(vec![
                    PrinterInfo::new("broken".into(), false),
                    PrinterInfo::new("working".into(), true),
                ])
            }
        }
        fn list_papers(&self, printer: &str) -> PrintResult<Vec<PaperInfo>> {
            if printer == "broken" {
                Err(PrintError::PaperNotFound("70 x 45 mm".into()))
            } else {
                Ok(vec![PaperInfo {
                    id: "256".into(),
                    name: "Label".into(),
                    width_mm: 70.0,
                    height_mm: 45.0,
                }])
            }
        }
        fn print_pdf(&self, _: &Path, _: &PrintOptions) -> PrintResult<PrintSubmission> {
            unreachable!()
        }
        fn print_raw(
            &self,
            _: &[u8],
            _: &super::super::RawPrintOptions,
        ) -> PrintResult<PrintSubmission> {
            unreachable!()
        }
    }

    #[test]
    fn retains_latest_events_in_order() {
        let mut events = VecDeque::new();
        for index in 0..CAPACITY + 2 {
            push(&mut events, json!(index));
        }
        assert_eq!(events.len(), CAPACITY);
        assert_eq!(events.front(), Some(&json!(2)));
        assert_eq!(events.back(), Some(&json!(CAPACITY + 1)));
    }

    #[test]
    fn exports_valid_report_even_when_printer_discovery_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        export_report(
            &path,
            &FailingBackend {
                discovery_failure: true,
            },
        )
        .unwrap();
        let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(report["printers"]["ok"], false);
        assert!(report["printers"]["error"]
            .as_str()
            .unwrap()
            .contains("test printer"));
        assert_eq!(report["paper_probes"], json!([]));
        assert!(report.get("app_version").is_some());
        assert!(report.get("config").is_none());
    }

    #[test]
    fn keeps_failure_evidence_and_continues_after_one_printer_fails() {
        let operation = uuid::Uuid::new_v4().to_string();
        record(
            &operation,
            "broken",
            "paper_match_failed",
            json!({"width_mm": 70, "height_mm": 45}),
        );
        let backend = FailingBackend {
            discovery_failure: false,
        };
        let report = collect_report(&backend);
        assert_eq!(report["paper_probes"][0]["papers"]["ok"], false);
        assert_eq!(
            report["paper_probes"][1]["papers"]["value"][0]["width_mm"],
            70.0
        );
        assert!(report["recent_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["operation"] == operation));
        let dir = tempfile::tempdir().unwrap();
        assert!(export_report(dir.path(), &backend).is_err());
    }
}
