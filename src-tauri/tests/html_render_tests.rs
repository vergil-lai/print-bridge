use print_bridge_lib::{
    html::{
        browser::{BrowserHtmlRenderer, BrowserLocator},
        proxy::FilteringProxy,
        resource_policy::ResourcePolicy,
        HtmlRenderError, HtmlRenderRequest, HtmlRenderer, HtmlSource,
    },
    protocol::EffectivePaper,
};
use std::{io::ErrorKind, net::TcpListener};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PUBLIC_ASSETS_HTML: &str = include_str!("fixtures/html/public-assets.html");

#[test]
#[ignore = "requires an installed Chromium-family browser; run explicitly for the browser smoke gate"]
fn browser_renders_css_image_font_and_javascript_fixture_to_pdf() {
    let browser = match BrowserLocator::new().find() {
        Ok(browser) => browser,
        Err(HtmlRenderError::RendererUnavailable { searched }) => {
            eprintln!(
                "SKIP browser smoke: no compatible Chromium-family browser was found (searched: {searched:?})"
            );
            return;
        }
        Err(error) => panic!("browser probe failed unexpectedly: {error}"),
    };
    eprintln!(
        "running browser smoke with {} at {}",
        browser.label,
        browser.path.display()
    );

    let directory = tempfile::tempdir().unwrap();
    let output_path = directory.path().join("public-assets.pdf");
    let renderer = BrowserHtmlRenderer::new(ResourcePolicy::system());
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(renderer.render(HtmlRenderRequest {
            source: HtmlSource::Inline(PUBLIC_ASSETS_HTML.to_string()),
            paper: EffectivePaper {
                width_mm: 100.0,
                height_mm: 150.0,
            },
            wait_ms: 150,
            output_path: output_path.clone(),
        }))
        .unwrap();

    assert_eq!(result.output_path, output_path);
    let pdf = std::fs::read(&output_path).unwrap();
    assert!(pdf.starts_with(b"%PDF-"), "PDF header is missing");
    assert!(pdf.len() > 5, "PDF is empty");
}

#[test]
#[ignore = "requires an installed Chromium-family browser; run explicitly for the browser loopback gate"]
fn browser_reports_a_loopback_page_resource_without_connecting_to_it() {
    let browser = match BrowserLocator::new().find() {
        Ok(browser) => browser,
        Err(HtmlRenderError::RendererUnavailable { searched }) => {
            eprintln!(
                "SKIP browser loopback gate: no compatible Chromium-family browser was found (searched: {searched:?})"
            );
            return;
        }
        Err(error) => panic!("browser probe failed unexpectedly: {error}"),
    };
    eprintln!(
        "running browser loopback gate with {} at {}",
        browser.label,
        browser.path.display()
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let blocked_url = format!("http://{}/private.png", listener.local_addr().unwrap());
    let directory = tempfile::tempdir().unwrap();
    let renderer = BrowserHtmlRenderer::new(ResourcePolicy::system());
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(renderer.render(HtmlRenderRequest {
            source: HtmlSource::Inline(format!("<img src=\"{blocked_url}\" />")),
            paper: EffectivePaper {
                width_mm: 100.0,
                height_mm: 150.0,
            },
            wait_ms: 150,
            output_path: directory.path().join("loopback.pdf"),
        }));

    let listener_result = listener.accept().map(|_| ()).map_err(|error| error.kind());
    match result {
        Err(HtmlRenderError::BlockedResource { resource }) => assert_eq!(resource, blocked_url),
        result => panic!(
            "expected blocked loopback page resource, got {result:?}; listener result: {listener_result:?}"
        ),
    }
    assert_eq!(listener_result, Err(ErrorKind::WouldBlock));
}

#[tokio::test]
async fn loopback_html_resource_is_rejected_before_the_server_is_reached() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let blocked_url = format!("http://{}/private.png", listener.local_addr().unwrap());
    let html = format!("<img src=\"{blocked_url}\" alt=\"private resource\" />");

    let proxy = FilteringProxy::start(ResourcePolicy::system(), Some(html.clone()))
        .await
        .unwrap();
    let page_url = proxy.target_url(HtmlSource::Inline(html));
    let proxy_address = format!(
        "{}:{}",
        proxy.proxy_url().host_str().unwrap(),
        proxy.proxy_url().port().unwrap()
    );
    let mut stream = tokio::net::TcpStream::connect(proxy_address).await.unwrap();
    stream
        .write_all(
            format!(
                "GET {blocked_url} HTTP/1.1\r\nHost: {}\r\nReferer: {page_url}\r\nConnection: close\r\n\r\n",
                listener.local_addr().unwrap()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();

    assert!(response.starts_with(b"HTTP/1.1 403"));
    assert_eq!(proxy.rejected_resource(), Some(blocked_url));
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
}
