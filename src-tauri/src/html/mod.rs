pub mod browser;

pub mod resource_policy;

pub mod proxy;

use crate::protocol::EffectivePaper;
use std::{future::Future, path::PathBuf, pin::Pin};
use thiserror::Error;
use url::Url;

/// 浏览器渲染的 HTML 来源。
#[derive(Debug, Clone)]
pub enum HtmlSource {
    Url(Url),
    Inline(String),
}

/// 浏览器渲染 HTML 所需的输入。
#[derive(Debug, Clone)]
pub struct HtmlRenderRequest {
    pub source: HtmlSource,
    pub paper: EffectivePaper,
    pub wait_ms: u64,
    pub output_path: PathBuf,
}

/// HTML 渲染成功后的 PDF 结果。
#[derive(Debug, Clone)]
pub struct HtmlRenderResult {
    pub renderer: &'static str,
    pub output_path: PathBuf,
}

/// HTML 渲染的异步结果。
pub type HtmlRenderFuture =
    Pin<Box<dyn Future<Output = Result<HtmlRenderResult, HtmlRenderError>> + Send>>;

/// 将受限 HTML 来源渲染为 PDF 的后端。
pub trait HtmlRenderer: Send + Sync {
    fn render(&self, request: HtmlRenderRequest) -> HtmlRenderFuture;
}

/// HTML 渲染阶段发生的错误。
#[derive(Debug, Error)]
pub enum HtmlRenderError {
    #[error("no installed Chromium-family browser is available; searched: {searched:?}")]
    RendererUnavailable { searched: Vec<String> },
    #[error("blocked HTML resource: {resource}")]
    BlockedResource { resource: String },
    #[error("browser navigation failed: {message}")]
    Navigation { message: String },
    #[error("browser rendering timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("browser PDF export failed: {message}")]
    PdfExport { message: String },
    #[error("invalid proxy request: {reason}")]
    InvalidProxyRequest { reason: String },
    #[error("HTML proxy I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTML proxy HTTP error: {0}")]
    Http(#[from] hyper::Error),
}
