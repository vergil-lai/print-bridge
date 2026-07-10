//! 过滤代理实现。

use crate::html::{resource_policy::ResourcePolicy, HtmlRenderError, HtmlSource};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{
    body::Incoming,
    client::conn::http1,
    header::{CONTENT_TYPE, HOST, ORIGIN, REFERER},
    server::conn::http1 as server_http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};
use url::Url;
use uuid::Uuid;

type ProxyBody = BoxBody<Bytes, io::Error>;

#[derive(Debug, Clone)]
pub(crate) struct RejectedResourceTracker {
    rejected_resource: Arc<Mutex<Option<String>>>,
    page_url: Arc<Mutex<Option<Url>>>,
}

impl RejectedResourceTracker {
    fn new() -> Self {
        Self {
            rejected_resource: Arc::new(Mutex::new(None)),
            page_url: Arc::new(Mutex::new(None)),
        }
    }

    fn set_page_url(&self, page_url: &Url) {
        if let Ok(mut current_page_url) = self.page_url.lock() {
            *current_page_url = Some(page_url.clone());
        }
    }

    fn belongs_to_page(&self, request: &Request<Incoming>) -> bool {
        let Some(page_url) = self.page_url.lock().ok().and_then(|url| url.clone()) else {
            return false;
        };
        [REFERER, ORIGIN].iter().any(|header| {
            request
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| Url::parse(value).ok())
                .is_some_and(|value| value.origin() == page_url.origin())
        })
    }

    fn record(&self, resource: &str, belongs_to_page: bool) {
        if !belongs_to_page {
            return;
        }
        if let Ok(mut rejected_resource) = self.rejected_resource.lock() {
            rejected_resource.get_or_insert_with(|| resource.to_string());
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut rejected_resource) = self.rejected_resource.lock() {
            *rejected_resource = None;
        }
    }

    fn get(&self) -> Option<String> {
        self.rejected_resource.lock().ok()?.clone()
    }
}

/// 只可连接资源策略已经批准地址的连接器。
pub trait ApprovedConnector: Send + Sync {
    /// 连接资源策略已批准的套接字地址。
    fn connect<'a>(
        &'a self,
        approved_address: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<TcpStream>> + Send + 'a>>;
}

struct TcpConnector;

impl ApprovedConnector for TcpConnector {
    fn connect<'a>(
        &'a self,
        approved_address: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<TcpStream>> + Send + 'a>> {
        Box::pin(async move { TcpStream::connect(approved_address).await })
    }
}

struct ProxyState {
    policy: ResourcePolicy,
    connector: Arc<dyn ApprovedConnector>,
    inline: Option<InlineRoute>,
    rejected_resource: RejectedResourceTracker,
}

#[derive(Clone)]
struct InlineRoute {
    authority: String,
    body: String,
}

/// 只允许公开 HTTP(S) 资源的本机过滤代理。
pub struct FilteringProxy {
    proxy_url: Url,
    inline_url: Option<Url>,
    rejected_resource: RejectedResourceTracker,
    shutdown: Option<oneshot::Sender<()>>,
    accept_loop: Option<JoinHandle<()>>,
}

impl FilteringProxy {
    /// 在随机环回端口启动过滤代理。
    pub async fn start(
        policy: ResourcePolicy,
        inline_html: Option<String>,
    ) -> Result<Self, HtmlRenderError> {
        Self::start_with(policy, inline_html, Arc::new(TcpConnector)).await
    }

    #[cfg(test)]
    async fn start_with_connector(
        policy: ResourcePolicy,
        inline_html: Option<String>,
        connector: Arc<dyn ApprovedConnector>,
    ) -> Result<Self, HtmlRenderError> {
        Self::start_with(policy, inline_html, connector).await
    }

    async fn start_with(
        policy: ResourcePolicy,
        inline_html: Option<String>,
        connector: Arc<dyn ApprovedConnector>,
    ) -> Result<Self, HtmlRenderError> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let inline = inline_html.map(|body| {
            let authority = format!("{}.printbridge.invalid", Uuid::new_v4().simple());
            InlineRoute { authority, body }
        });
        let inline_url = inline
            .as_ref()
            .map(|route| Url::parse(&format!("http://{}/", route.authority)))
            .transpose()
            .map_err(|error| HtmlRenderError::InvalidProxyRequest {
                reason: error.to_string(),
            })?;
        let rejected_resource = RejectedResourceTracker::new();
        let state = Arc::new(ProxyState {
            policy,
            connector,
            inline,
            rejected_resource: rejected_resource.clone(),
        });
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let accept_loop = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => {
                            let state = state.clone();
                            tokio::spawn(async move {
                                let service = service_fn(move |request| state.clone().handle(request));
                                let _ = server_http1::Builder::new()
                                    .serve_connection(TokioIo::new(stream), service)
                                    .with_upgrades()
                                    .await;
                            });
                        }
                        Err(_) => break,
                    },
                }
            }
        });

        Ok(Self {
            proxy_url: Url::parse(&format!("http://{address}"))
                .expect("loopback proxy URL must be valid"),
            inline_url,
            rejected_resource,
            shutdown: Some(shutdown),
            accept_loop: Some(accept_loop),
        })
    }

    /// 返回浏览器应配置的本机代理地址。
    pub fn proxy_url(&self) -> &Url {
        &self.proxy_url
    }

    /// 返回内联 HTML 的随机 synthetic URL。
    pub fn inline_url(&self) -> Option<&Url> {
        self.inline_url.as_ref()
    }

    /// 返回指定 HTML 来源的浏览目标 URL。
    pub fn target_url(&self, source: HtmlSource) -> Url {
        let target_url = match source {
            HtmlSource::Url(url) => url,
            HtmlSource::Inline(_) => self
                .inline_url
                .clone()
                .expect("inline HTML requires a proxy inline route"),
        };
        self.rejected_resource.set_page_url(&target_url);
        target_url
    }

    /// 返回本次渲染中被资源策略拒绝的第一个资源 URL。
    pub fn rejected_resource(&self) -> Option<String> {
        self.rejected_resource.get()
    }

    pub(crate) fn rejection_tracker(&self) -> RejectedResourceTracker {
        self.rejected_resource.clone()
    }

    #[cfg(test)]
    fn clear_rejected_resource(&self) {
        self.rejected_resource.clear();
    }
}

impl Drop for FilteringProxy {
    fn drop(&mut self) {
        if let Some(accept_loop) = self.accept_loop.take() {
            accept_loop.abort();
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl ProxyState {
    async fn handle(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> Result<Response<ProxyBody>, Infallible> {
        let belongs_to_page = self.rejected_resource.belongs_to_page(&request);
        let response = if request.method() == Method::CONNECT {
            self.clone().handle_connect(request).await
        } else if self.is_inline_request(&request) {
            Ok(inline_response(
                self.inline.as_ref().expect("route checked"),
            ))
        } else if self.is_inline_favicon_request(&request) {
            Ok(no_content_response())
        } else {
            self.clone().handle_http(request).await
        };
        Ok(match response {
            Ok(response) => response,
            Err(error) => {
                self.record_rejected_resource(&error, belongs_to_page);
                error_response(error)
            }
        })
    }

    fn record_rejected_resource(&self, error: &HtmlRenderError, belongs_to_page: bool) {
        let HtmlRenderError::BlockedResource { resource } = error else {
            return;
        };
        self.rejected_resource.record(resource, belongs_to_page);
    }

    fn is_inline_request(&self, request: &Request<Incoming>) -> bool {
        let Some(route) = &self.inline else {
            return false;
        };
        request_authority(request).is_some_and(|authority| authority == route.authority)
            && request.uri().path() == "/"
            && request.uri().query().is_none()
    }

    fn is_inline_favicon_request(&self, request: &Request<Incoming>) -> bool {
        let Some(route) = &self.inline else {
            return false;
        };
        request_authority(request).is_some_and(|authority| authority == route.authority)
            && request.uri().path() == "/favicon.ico"
            && request.uri().query().is_none()
    }

    fn is_inline_connect_authority(&self, authority: &str) -> bool {
        let Some(route) = &self.inline else {
            return false;
        };
        authority == route.authority || authority == format!("{}:80", route.authority)
    }

    async fn handle_connect(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> Result<Response<ProxyBody>, HtmlRenderError> {
        let authority = request
            .uri()
            .authority()
            .ok_or_else(|| invalid_request("CONNECT target must be an authority"))?
            .as_str();
        if self.is_inline_connect_authority(authority) {
            let route = self
                .inline
                .as_ref()
                .expect("inline CONNECT requires an inline route")
                .clone();
            tokio::spawn(async move {
                if let Ok(upgraded) = hyper::upgrade::on(request).await {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let route = route.clone();
                        async move {
                            let is_page = request
                                .headers()
                                .get(HOST)
                                .and_then(|host| host.to_str().ok())
                                .is_some_and(|host| host == route.authority)
                                && request.uri().path() == "/"
                                && request.uri().query().is_none();
                            Ok::<_, Infallible>(if is_page {
                                inline_response(&route)
                            } else {
                                error_response(invalid_request(
                                    "inline CONNECT tunnel only serves its synthetic page",
                                ))
                            })
                        }
                    });
                    let upgraded = TokioIo::new(upgraded);
                    let _ = server_http1::Builder::new()
                        .serve_connection(TokioIo::new(upgraded), service)
                        .with_upgrades()
                        .await;
                }
            });
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .body(empty_body())
                .expect("valid CONNECT response"));
        }
        let url = Url::parse(&format!("https://{authority}/")).map_err(|error| {
            HtmlRenderError::InvalidProxyRequest {
                reason: error.to_string(),
            }
        })?;
        let target = self.policy.resolve_public_target(&url).await?;
        let upstream = self.connector.connect(target.address).await?;
        tokio::spawn(async move {
            if let Ok(upgraded) = hyper::upgrade::on(request).await {
                let mut client = TokioIo::new(upgraded);
                let mut upstream = upstream;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            }
        });
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(empty_body())
            .expect("valid CONNECT response"))
    }

    async fn handle_http(
        self: Arc<Self>,
        mut request: Request<Incoming>,
    ) -> Result<Response<ProxyBody>, HtmlRenderError> {
        let url = proxy_request_url(&request)?;
        if url.scheme() != "http" {
            return Err(invalid_request("HTTPS resources must use CONNECT"));
        }
        let target = self.policy.resolve_public_target(&url).await?;
        let upstream = self.connector.connect(target.address).await?;
        let origin_form = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        };
        *request.uri_mut() =
            origin_form
                .parse()
                .map_err(|error: hyper::http::uri::InvalidUri| {
                    HtmlRenderError::InvalidProxyRequest {
                        reason: error.to_string(),
                    }
                })?;
        if !request.headers().contains_key(HOST) {
            request.headers_mut().insert(
                HOST,
                target.authority.parse().map_err(
                    |error: hyper::http::header::InvalidHeaderValue| {
                        HtmlRenderError::InvalidProxyRequest {
                            reason: error.to_string(),
                        }
                    },
                )?,
            );
        }
        let (mut sender, connection) = http1::handshake(TokioIo::new(upstream)).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let response = sender.send_request(request).await?;
        Ok(response.map(|body| body.map_err(io::Error::other).boxed()))
    }
}

/// 返回代理请求的目标 authority，兼容 absolute-form 与 origin-form 请求行。
fn request_authority(request: &Request<Incoming>) -> Option<&str> {
    request
        .uri()
        .authority()
        .map(|authority| authority.as_str())
        .or_else(|| {
            request
                .headers()
                .get(HOST)
                .and_then(|host| host.to_str().ok())
        })
}

/// 将 HTTP 代理请求还原为绝对 URL，兼容 WebView 使用的 origin-form 请求行。
fn proxy_request_url(request: &Request<Incoming>) -> Result<Url, HtmlRenderError> {
    if request.uri().scheme().is_some() {
        return Url::parse(&request.uri().to_string())
            .map_err(|_| invalid_request("proxy request URL is invalid"));
    }

    let authority = request_authority(request)
        .ok_or_else(|| invalid_request("origin-form proxy request is missing Host"))?;
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    Url::parse(&format!("http://{authority}{path_and_query}"))
        .map_err(|_| invalid_request("origin-form proxy request URL is invalid"))
}

fn inline_response(route: &InlineRoute) -> Response<ProxyBody> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(full_body(route.body.clone()))
        .expect("valid inline HTML response")
}

fn no_content_response() -> Response<ProxyBody> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(empty_body())
        .expect("valid no-content response")
}

fn error_response(error: HtmlRenderError) -> Response<ProxyBody> {
    let status = match error {
        HtmlRenderError::BlockedResource { .. } => StatusCode::FORBIDDEN,
        _ => StatusCode::BAD_GATEWAY,
    };
    Response::builder()
        .status(status)
        .body(empty_body())
        .expect("valid proxy error response")
}

fn invalid_request(reason: impl Into<String>) -> HtmlRenderError {
    HtmlRenderError::InvalidProxyRequest {
        reason: reason.into(),
    }
}

fn empty_body() -> ProxyBody {
    full_body(Bytes::new())
}

fn full_body(body: impl Into<Bytes>) -> ProxyBody {
    Full::new(body.into())
        .map_err(|never| match never {})
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::{ApprovedConnector, FilteringProxy};
    use crate::html::{
        resource_policy::{HostResolver, ResourcePolicy},
        HtmlSource,
    };
    use std::{
        collections::HashMap,
        future::Future,
        io,
        net::{IpAddr, SocketAddr},
        pin::Pin,
        sync::{Arc, Mutex},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    struct FakeResolver {
        addresses: HashMap<String, Vec<IpAddr>>,
    }

    impl HostResolver for FakeResolver {
        fn resolve<'a>(
            &'a self,
            host: &'a str,
            port: u16,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>> {
            Box::pin(async move {
                Ok(self
                    .addresses
                    .get(host)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|address| SocketAddr::new(address, port))
                    .collect())
            })
        }
    }

    struct TestConnector {
        upstream: SocketAddr,
        connections: Mutex<usize>,
    }

    async fn read_until_header_end(stream: &mut TcpStream, bytes: &mut Vec<u8>) {
        let mut buffer = [0_u8; 1024];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0, "proxy closed the tunnel before responding");
            bytes.extend_from_slice(&buffer[..count]);
        }
    }

    impl TestConnector {
        fn new(upstream: SocketAddr) -> Self {
            Self {
                upstream,
                connections: Mutex::new(0),
            }
        }

        fn connections(&self) -> usize {
            *self.connections.lock().unwrap()
        }
    }

    impl ApprovedConnector for TestConnector {
        fn connect<'a>(
            &'a self,
            _approved_address: SocketAddr,
        ) -> Pin<Box<dyn Future<Output = io::Result<TcpStream>> + Send + 'a>> {
            Box::pin(async move {
                *self.connections.lock().unwrap() += 1;
                TcpStream::connect(self.upstream).await
            })
        }
    }

    fn test_policy() -> ResourcePolicy {
        ResourcePolicy::new(Arc::new(FakeResolver {
            addresses: HashMap::from([
                (
                    "public.example.com".to_string(),
                    vec!["93.184.216.34".parse().unwrap()],
                ),
                (
                    "private.example.com".to_string(),
                    vec!["10.0.0.8".parse().unwrap()],
                ),
                (
                    "www.google.com".to_string(),
                    vec!["10.0.0.8".parse().unwrap()],
                ),
            ]),
        }))
    }

    async fn upstream(response: &'static str, request: Arc<Mutex<String>>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0_u8; 1024];
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            *request.lock().unwrap() = String::from_utf8(bytes).unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        address
    }

    #[tokio::test]
    async fn proxy_serves_inline_html_only_on_its_one_time_route() {
        let proxy = FilteringProxy::start(test_policy(), Some("<h1>ok</h1>".into()))
            .await
            .unwrap();
        let body = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.proxy_url().as_str()).unwrap())
            .build()
            .unwrap()
            .get(proxy.inline_url().unwrap().clone())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        assert_eq!(body, "<h1>ok</h1>");
    }

    #[tokio::test]
    async fn proxy_serves_an_empty_favicon_for_the_inline_page() {
        let proxy = FilteringProxy::start(test_policy(), Some("<h1>ok</h1>".into()))
            .await
            .unwrap();
        let favicon_url = proxy.inline_url().unwrap().join("favicon.ico").unwrap();
        let response = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.proxy_url().as_str()).unwrap())
            .build()
            .unwrap()
            .get(favicon_url)
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn proxy_forwards_http_with_the_original_host_header() {
        let request = Arc::new(Mutex::new(String::new()));
        let upstream = upstream(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            request.clone(),
        )
        .await;
        let connector = Arc::new(TestConnector::new(upstream));
        let proxy = FilteringProxy::start_with_connector(test_policy(), None, connector)
            .await
            .unwrap();

        let body = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.proxy_url().as_str()).unwrap())
            .build()
            .unwrap()
            .get("http://public.example.com:8080/invoice?copy=1")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        assert_eq!(body, "ok");
        let request = request.lock().unwrap().to_ascii_lowercase();
        assert!(request.contains("host: public.example.com:8080\r\n"));
    }

    #[tokio::test]
    async fn proxy_preserves_a_custom_original_host_header() {
        let request = Arc::new(Mutex::new(String::new()));
        let upstream = upstream(
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            request.clone(),
        )
        .await;
        let proxy = FilteringProxy::start_with_connector(
            test_policy(),
            None,
            Arc::new(TestConnector::new(upstream)),
        )
        .await
        .unwrap();
        let mut stream = TcpStream::connect(proxy.proxy_url().socket_addrs(|| None).unwrap()[0])
            .await
            .unwrap();
        stream
            .write_all(b"GET http://public.example.com:8080/invoice HTTP/1.1\r\nHost: invoice.virtual.example\r\n\r\n")
            .await
            .unwrap();
        let mut response = [0_u8; 128];
        let read = stream.read(&mut response).await.unwrap();

        assert!(std::str::from_utf8(&response[..read])
            .unwrap()
            .starts_with("HTTP/1.1 200"));
        assert!(request
            .lock()
            .unwrap()
            .to_ascii_lowercase()
            .contains("host: invoice.virtual.example\r\n"));
    }

    #[tokio::test]
    async fn proxy_rejects_private_destinations_before_connecting() {
        let connector = Arc::new(TestConnector::new("127.0.0.1:9".parse().unwrap()));
        let proxy = FilteringProxy::start_with_connector(test_policy(), None, connector.clone())
            .await
            .unwrap();
        let response = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.proxy_url().as_str()).unwrap())
            .build()
            .unwrap()
            .get("http://private.example.com/")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        assert_eq!(connector.connections(), 0);
    }

    #[tokio::test]
    async fn proxy_can_clear_a_rejection_recorded_before_page_navigation() {
        let connector = Arc::new(TestConnector::new("127.0.0.1:9".parse().unwrap()));
        let proxy = FilteringProxy::start_with_connector(
            test_policy(),
            Some("<main>page</main>".to_string()),
            connector,
        )
        .await
        .unwrap();
        let page_url = proxy.target_url(HtmlSource::Inline("<main>page</main>".to_string()));
        let response = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.proxy_url().as_str()).unwrap())
            .build()
            .unwrap()
            .get("http://private.example.com/")
            .header("referer", page_url.as_str())
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        assert_eq!(
            proxy.rejected_resource(),
            Some("http://private.example.com/".to_string())
        );
        proxy.clear_rejected_resource();
        assert_eq!(proxy.rejected_resource(), None);
    }

    #[tokio::test]
    async fn proxy_serves_inline_html_for_an_origin_form_proxy_request() {
        let proxy = FilteringProxy::start(test_policy(), Some("<main>inline page</main>".into()))
            .await
            .unwrap();
        let page_url = proxy.target_url(HtmlSource::Inline("<main>inline page</main>".into()));
        let mut stream = TcpStream::connect(proxy.proxy_url().socket_addrs(|| None).unwrap()[0])
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                    page_url.host_str().unwrap()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();

        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("<main>inline page</main>"));
    }

    #[tokio::test]
    async fn proxy_serves_inline_html_inside_an_http_connect_tunnel() {
        let proxy = FilteringProxy::start(test_policy(), Some("<main>inline page</main>".into()))
            .await
            .unwrap();
        let page_url = proxy.target_url(HtmlSource::Inline("<main>inline page</main>".into()));
        let mut stream = TcpStream::connect(proxy.proxy_url().socket_addrs(|| None).unwrap()[0])
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "CONNECT {}:80 HTTP/1.1\r\nHost: {}:80\r\n\r\n",
                    page_url.host_str().unwrap(),
                    page_url.host_str().unwrap()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        read_until_header_end(&mut stream, &mut response).await;
        assert!(std::str::from_utf8(&response)
            .unwrap()
            .starts_with("HTTP/1.1 200"));

        stream
            .write_all(
                format!(
                    "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                    page_url.host_str().unwrap()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        response.clear();
        stream.read_to_end(&mut response).await.unwrap();

        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("<main>inline page</main>"));
    }

    #[tokio::test]
    async fn proxy_does_not_record_an_unattributed_browser_background_request() {
        let connector = Arc::new(TestConnector::new("127.0.0.1:9".parse().unwrap()));
        let proxy = FilteringProxy::start_with_connector(
            test_policy(),
            Some("<main>page</main>".to_string()),
            connector,
        )
        .await
        .unwrap();
        proxy.target_url(HtmlSource::Inline("<main>page</main>".to_string()));
        let response = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.proxy_url().as_str()).unwrap())
            .build()
            .unwrap()
            .get("http://www.google.com/")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        assert_eq!(proxy.rejected_resource(), None);
    }

    #[tokio::test]
    async fn proxy_rejects_https_without_a_connect_tunnel() {
        let connector = Arc::new(TestConnector::new("127.0.0.1:9".parse().unwrap()));
        let proxy = FilteringProxy::start_with_connector(test_policy(), None, connector.clone())
            .await
            .unwrap();
        let mut stream = TcpStream::connect(proxy.proxy_url().socket_addrs(|| None).unwrap()[0])
            .await
            .unwrap();
        stream
            .write_all(
                b"GET https://public.example.com/ HTTP/1.1\r\nHost: public.example.com\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = [0_u8; 128];
        let read = stream.read(&mut response).await.unwrap();

        assert!(std::str::from_utf8(&response[..read])
            .unwrap()
            .starts_with("HTTP/1.1 502"));
        assert_eq!(connector.connections(), 0);
    }

    #[tokio::test]
    async fn proxy_revalidates_a_redirect_to_a_private_host() {
        let request = Arc::new(Mutex::new(String::new()));
        let upstream = upstream(
            "HTTP/1.1 302 Found\r\nLocation: http://private.example.com/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            request,
        )
        .await;
        let connector = Arc::new(TestConnector::new(upstream));
        let proxy = FilteringProxy::start_with_connector(test_policy(), None, connector.clone())
            .await
            .unwrap();
        let response = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.proxy_url().as_str()).unwrap())
            .build()
            .unwrap()
            .get("http://public.example.com/")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        assert_eq!(connector.connections(), 1);
    }

    #[tokio::test]
    async fn proxy_tunnels_connect_to_the_approved_address() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0_u8; 4];
            stream.read_exact(&mut bytes).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        });
        let proxy = FilteringProxy::start_with_connector(
            test_policy(),
            None,
            Arc::new(TestConnector::new(upstream)),
        )
        .await
        .unwrap();
        let mut stream = TcpStream::connect(proxy.proxy_url().socket_addrs(|| None).unwrap()[0])
            .await
            .unwrap();
        stream
            .write_all(
                b"CONNECT public.example.com:443 HTTP/1.1\r\nHost: public.example.com:443\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = vec![0_u8; 64];
        let read = stream.read(&mut response).await.unwrap();
        assert!(std::str::from_utf8(&response[..read])
            .unwrap()
            .starts_with("HTTP/1.1 200"));
        stream.write_all(b"ping").await.unwrap();
        let mut echoed = [0_u8; 4];
        stream.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"ping");
    }

    #[tokio::test]
    async fn dropping_proxy_shuts_down_its_listener() {
        let proxy = FilteringProxy::start(test_policy(), None).await.unwrap();
        let proxy_url = proxy.proxy_url().clone();
        let accept_loop = proxy.accept_loop.as_ref().unwrap().abort_handle();
        drop(proxy);
        tokio::task::yield_now().await;

        assert!(accept_loop.is_finished());
        assert!(
            TcpStream::connect(proxy_url.socket_addrs(|| None).unwrap()[0])
                .await
                .is_err()
        );
    }
}
