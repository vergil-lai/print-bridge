use print_bridge_lib::protocol::{
    is_allowed_origin, validate_file_url, validate_origin, ClientMessage, EffectivePaper,
    ErrorCode, JobStatus, JobValidationError, PrintJobInput, SupportedFormat,
};
use std::str::FromStr;

#[test]
fn supported_format_accepts_pdf_image_and_legacy_image_subtypes() {
    assert_eq!(
        SupportedFormat::from_str("pdf").unwrap(),
        SupportedFormat::Pdf
    );
    assert_eq!(
        SupportedFormat::from_str("image").unwrap(),
        SupportedFormat::Image
    );
    assert_eq!(
        SupportedFormat::from_str("png").unwrap(),
        SupportedFormat::Png
    );
    assert_eq!(
        SupportedFormat::from_str("jpg").unwrap(),
        SupportedFormat::Jpg
    );
    assert_eq!(
        SupportedFormat::from_str("jpeg").unwrap(),
        SupportedFormat::Jpeg
    );
    assert_eq!(
        SupportedFormat::from_str("docx").unwrap(),
        SupportedFormat::Docx
    );
    assert_eq!(
        SupportedFormat::from_str("xlsx").unwrap(),
        SupportedFormat::Xlsx
    );
    assert_eq!(
        SupportedFormat::from_str("pptx").unwrap(),
        SupportedFormat::Pptx
    );
    assert_eq!(
        SupportedFormat::from_str("html").unwrap(),
        SupportedFormat::Html
    );
    assert_eq!(
        SupportedFormat::from_str("raw-html").unwrap(),
        SupportedFormat::RawHtml
    );
}

#[test]
fn parses_html_and_raw_html_jobs() {
    let url_job: PrintJobInput = serde_json::from_str(
        r#"{
            "job_id":"html-url",
            "format":"html",
            "file_url":"https://example.com/invoice/1",
            "wait_ms":1500
        }"#,
    )
    .unwrap();
    assert_eq!(url_job.format, SupportedFormat::Html);
    assert_eq!(url_job.wait_ms, Some(1500));

    let inline_job: PrintJobInput = serde_json::from_str(
        r#"{
            "job_id":"html-inline",
            "format":"raw-html",
            "html":"<main>invoice</main>"
        }"#,
    )
    .unwrap();
    assert_eq!(inline_job.format, SupportedFormat::RawHtml);
    assert_eq!(inline_job.html.as_deref(), Some("<main>invoice</main>"));
}

#[test]
fn validates_html_field_combinations_and_wait_range() {
    let mut job = PrintJobInput {
        job_id: "html".into(),
        format: SupportedFormat::Html,
        printer_name: None,
        file_url: Some("https://example.com/page".into()),
        data_base64: None,
        html: None,
        wait_ms: Some(30_000),
        copies: Some(1),
        paper: None,
    };
    assert!(job.validate_for_acceptance(10).is_ok());
    job.wait_ms = Some(30_001);
    assert_eq!(
        job.validate_for_acceptance(10),
        Err(JobValidationError::HtmlWaitOutOfRange)
    );

    job.wait_ms = None;
    job.file_url = None;
    assert_eq!(
        job.validate_for_acceptance(10),
        Err(JobValidationError::MissingHtmlFileUrl)
    );

    job.file_url = Some("https://example.com/page".into());
    job.html = Some("<main>forbidden</main>".into());
    assert_eq!(
        job.validate_for_acceptance(10),
        Err(JobValidationError::HtmlInlineNotAllowed)
    );

    job.html = None;
    job.data_base64 = Some("PG1haW4+PC9tYWluPg==".into());
    assert_eq!(
        job.validate_for_acceptance(10),
        Err(JobValidationError::HtmlDataBase64NotAllowed)
    );
}

#[test]
fn html_file_jobs_require_absolute_http_or_https_urls() {
    let mut job = PrintJobInput {
        job_id: "html-url-scheme".into(),
        format: SupportedFormat::Html,
        printer_name: None,
        file_url: Some("https://example.com/invoice".into()),
        data_base64: None,
        html: None,
        wait_ms: None,
        copies: Some(1),
        paper: None,
    };

    assert!(job.validate_for_acceptance(1).is_ok());

    // Host/IP decisions intentionally remain with ResourcePolicy during rendering.
    job.file_url = Some("http://127.0.0.1:8080/invoice".into());
    assert!(job.validate_for_acceptance(1).is_ok());

    for file_url in [
        "file:///tmp/invoice.html",
        "data:text/html,<main>invoice</main>",
        "not-a-url",
        "https://",
    ] {
        job.file_url = Some(file_url.into());
        assert_eq!(
            job.validate_for_acceptance(1),
            Err(JobValidationError::InvalidHtmlFileUrl),
            "{file_url} should be rejected before it reaches the queue"
        );
    }
}

#[test]
fn validates_raw_html_and_non_html_field_combinations() {
    let mut raw_html = PrintJobInput {
        job_id: "raw-html".into(),
        format: SupportedFormat::RawHtml,
        printer_name: None,
        file_url: None,
        data_base64: None,
        html: None,
        wait_ms: Some(0),
        copies: Some(1),
        paper: None,
    };
    raw_html.html = Some("<main>invoice</main>".into());
    assert!(raw_html.validate_for_acceptance(1).is_ok());

    raw_html.html = None;
    assert_eq!(
        raw_html.validate_for_acceptance(1),
        Err(JobValidationError::MissingRawHtml)
    );

    raw_html.html = Some("   ".into());
    assert_eq!(
        raw_html.validate_for_acceptance(1),
        Err(JobValidationError::MissingRawHtml)
    );

    raw_html.html = Some("<main>invoice</main>".into());
    raw_html.file_url = Some("https://example.com/invoice".into());
    assert_eq!(
        raw_html.validate_for_acceptance(1),
        Err(JobValidationError::RawHtmlFileUrlNotAllowed)
    );

    raw_html.file_url = None;
    raw_html.data_base64 = Some("PG1haW4+PC9tYWluPg==".into());
    assert_eq!(
        raw_html.validate_for_acceptance(1),
        Err(JobValidationError::HtmlDataBase64NotAllowed)
    );

    raw_html.data_base64 = None;
    raw_html.html = Some("x".repeat(1024 * 1024 + 1));
    assert_eq!(
        raw_html.validate_for_acceptance(1),
        Err(JobValidationError::FileTooLarge)
    );

    let mut pdf = PrintJobInput {
        job_id: "pdf".into(),
        format: SupportedFormat::Pdf,
        printer_name: None,
        file_url: Some("https://example.com/document.pdf".into()),
        data_base64: None,
        html: Some("<main>forbidden</main>".into()),
        wait_ms: None,
        copies: Some(1),
        paper: None,
    };
    assert_eq!(
        pdf.validate_for_acceptance(1),
        Err(JobValidationError::NonHtmlHtmlNotAllowed)
    );

    pdf.html = None;
    pdf.wait_ms = Some(1);
    assert_eq!(
        pdf.validate_for_acceptance(1),
        Err(JobValidationError::NonHtmlWaitNotAllowed)
    );

    let mut raw = PrintJobInput {
        job_id: "raw".into(),
        format: SupportedFormat::Raw,
        printer_name: None,
        file_url: None,
        data_base64: Some("XlhB".into()),
        html: Some("<main>forbidden</main>".into()),
        wait_ms: None,
        copies: None,
        paper: None,
    };
    assert_eq!(
        raw.validate_for_acceptance(1),
        Err(JobValidationError::NonHtmlHtmlNotAllowed)
    );

    raw.html = None;
    raw.wait_ms = Some(1);
    assert_eq!(
        raw.validate_for_acceptance(1),
        Err(JobValidationError::NonHtmlWaitNotAllowed)
    );
}

#[test]
fn is_allowed_origin_requires_exact_origin_match() {
    let allowed = vec![
        "http://localhost:5173".to_string(),
        "https://app.example.com".to_string(),
    ];

    assert!(is_allowed_origin(Some("http://localhost:5173"), &allowed));
    assert!(is_allowed_origin(Some("https://app.example.com"), &allowed));

    assert!(!is_allowed_origin(None, &allowed));
    assert!(!is_allowed_origin(Some("null"), &allowed));
    assert!(!is_allowed_origin(
        Some("https://app.example.com/print"),
        &allowed
    ));
    assert!(!is_allowed_origin(
        Some("https://APP.example.com"),
        &allowed
    ));
}

#[test]
fn validate_origin_requires_http_or_https_origin_without_path() {
    assert!(validate_origin("http://localhost:5173").is_ok());
    assert!(validate_origin("https://app.example.com").is_ok());

    assert!(validate_origin("http").is_err());
    assert!(validate_origin("Asdf").is_err());
    assert!(validate_origin("ftp://app.example.com").is_err());
    assert!(validate_origin("https://app.example.com/print").is_err());
    assert!(validate_origin("https://app.example.com?from=settings").is_err());
    assert!(validate_origin("https://APP.example.com").is_err());
}

#[test]
fn validate_file_url_allows_http_https_and_pdf_data_urls() {
    assert!(validate_file_url("http://example.com/file.pdf").is_ok());
    assert!(validate_file_url("https://example.com/file.pdf").is_ok());
    assert!(validate_file_url("data:application/pdf;base64,JVBERi0xLjcK").is_ok());
    assert!(
        validate_file_url("data:application/pdf;filename=label.pdf;base64,JVBERi0xLjcK").is_ok()
    );

    assert!(validate_file_url("file:///tmp/file.pdf").is_err());
    assert!(validate_file_url("ftp://example.com/file.pdf").is_err());
    assert!(validate_file_url("/tmp/file.pdf").is_err());
    assert!(validate_file_url("data:text/html;base64,PGgxPkxhYmVsPC9oMT4=").is_err());
    assert!(validate_file_url("data:application/pdf,JVBERi0xLjcK").is_err());
}

#[test]
fn effective_paper_requires_positive_dimensions() {
    assert!(EffectivePaper {
        width_mm: 210.0,
        height_mm: 297.0,
    }
    .validate()
    .is_ok());

    assert!(EffectivePaper {
        width_mm: 0.0,
        height_mm: 297.0,
    }
    .validate()
    .is_err());
    assert!(EffectivePaper {
        width_mm: 210.0,
        height_mm: -1.0,
    }
    .validate()
    .is_err());
}

#[test]
fn print_job_input_requires_job_id_but_paper_is_optional() {
    let json = r#"{
        "job_id": "job-1",
        "file_url": "https://example.com/document.pdf",
        "format": "pdf",
        "copies": 1
    }"#;

    let job: PrintJobInput = serde_json::from_str(json).unwrap();

    assert_eq!(job.job_id, "job-1");
    assert_eq!(
        job.file_url.as_deref(),
        Some("https://example.com/document.pdf")
    );
    assert_eq!(job.format, SupportedFormat::Pdf);
    assert_eq!(job.copies, Some(1));
    assert!(job.paper.is_none());
}

#[test]
fn parses_printer_and_queue_query_messages() {
    let printers: ClientMessage = serde_json::from_str(
        r#"{
            "type": "get_printers_list",
            "request_id": "REQ-PRINTERS"
        }"#,
    )
    .unwrap();
    assert_eq!(
        printers,
        ClientMessage::GetPrintersList {
            request_id: "REQ-PRINTERS".to_string(),
        }
    );

    let printer_info: ClientMessage = serde_json::from_str(
        r#"{
            "type": "get_printer_info",
            "request_id": "REQ-INFO",
            "printer_name": "Zebra ZD421"
        }"#,
    )
    .unwrap();
    assert_eq!(
        printer_info,
        ClientMessage::GetPrinterInfo {
            request_id: "REQ-INFO".to_string(),
            printer_name: "Zebra ZD421".to_string(),
        }
    );

    let queue: ClientMessage = serde_json::from_str(
        r#"{
            "type": "get_print_queue",
            "request_id": "REQ-QUEUE"
        }"#,
    )
    .unwrap();
    assert_eq!(
        queue,
        ClientMessage::GetPrintQueue {
            request_id: "REQ-QUEUE".to_string(),
        }
    );
}

#[test]
fn print_job_input_rejects_missing_job_id() {
    let json = r#"{
        "file_url": "https://example.com/document.pdf",
        "format": "pdf",
        "copies": 1
    }"#;

    assert!(serde_json::from_str::<PrintJobInput>(json).is_err());
}

#[test]
fn client_message_print_parses_with_job_id() {
    let json = r#"{
        "type": "print",
        "request_id": "request-1",
        "job_id": "job-1",
        "file_url": "https://example.com/document.pdf",
        "format": "pdf",
        "copies": 2
    }"#;

    let message: ClientMessage = serde_json::from_str(json).unwrap();

    match message {
        ClientMessage::Print { request_id, job } => {
            assert_eq!(request_id, "request-1");
            assert_eq!(job.job_id, "job-1");
            assert_eq!(job.format, SupportedFormat::Pdf);
            assert_eq!(job.copies, Some(2));
            assert!(job.paper.is_none());
        }
        other => panic!("expected print message, got {other:?}"),
    }
}

#[test]
fn error_code_serializes_as_screaming_snake_case() {
    let json = serde_json::to_string(&ErrorCode::InvalidMessage).unwrap();

    assert_eq!(json, r#""INVALID_MESSAGE""#);
}

#[test]
fn job_status_submitted_serializes_as_submitted() {
    let json = serde_json::to_string(&JobStatus::Submitted).unwrap();

    assert_eq!(json, r#""submitted""#);
}

#[test]
fn job_status_completed_deserializes_from_completed() {
    let status: JobStatus = serde_json::from_str(r#""completed""#).unwrap();

    assert_eq!(status, JobStatus::Completed);
}

#[test]
fn job_status_unknown_deserializes_from_unknown() {
    let status: JobStatus = serde_json::from_str(r#""unknown""#).unwrap();

    assert_eq!(status, JobStatus::Unknown);
}
