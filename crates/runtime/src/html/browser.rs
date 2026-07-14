use crate::{
    html::{
        proxy::{FilteringProxy, RejectedResourceTracker},
        resource_policy::ResourcePolicy,
        HtmlRenderError, HtmlRenderFuture, HtmlRenderRequest, HtmlRenderResult, HtmlRenderer,
        HtmlSource,
    },
    protocol::EffectivePaper,
};
use headless_chrome::{types::PrintToPdfOptions, Browser, LaunchOptionsBuilder};
use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use url::Url;

const VERSION_CHECK_TIMEOUT: Duration = Duration::from_secs(2);
const RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_CDP_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const FORCE_LOOPBACK_THROUGH_PROXY: &str = "--proxy-bypass-list=<-loopback>";

fn chrome_proxy_server(proxy_url: &Url) -> String {
    let host = proxy_url
        .host_str()
        .expect("the filtering proxy URL always has a host");
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let port = proxy_url
        .port_or_known_default()
        .expect("the filtering proxy URL always has a port");
    format!("{}://{host}:{port}", proxy_url.scheme())
}

/// 已发现浏览器的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKind {
    Edge,
    Chrome,
    Chromium,
}

impl BrowserKind {
    fn label(self) -> &'static str {
        match self {
            Self::Edge => "edge",
            Self::Chrome => "chrome",
            Self::Chromium => "chromium",
        }
    }
}

/// 可用于启动渲染器的浏览器可执行文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserExecutable {
    pub kind: BrowserKind,
    pub path: PathBuf,
    pub label: &'static str,
}

/// 浏览器搜索的目标操作系统，方便测试候选顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Windows,
    MacOs,
    Linux,
}

impl TargetOs {
    fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Self::Linux
        }
    }
}

/// 操作系统文件和 PATH 探测接口。
pub trait BrowserProbe: Send + Sync {
    fn is_file(&self, path: &Path) -> bool;
    fn find_in_path(&self, command: &str) -> Option<PathBuf>;
    fn has_version(&self, path: &Path) -> bool;
}

struct SystemBrowserProbe;

impl BrowserProbe for SystemBrowserProbe {
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn find_in_path(&self, command: &str) -> Option<PathBuf> {
        let path = env::var_os("PATH")?;
        env::split_paths(&path)
            .map(|directory| directory.join(command))
            .find(|candidate| candidate.is_file())
    }

    fn has_version(&self, path: &Path) -> bool {
        let Ok(mut child) = Command::new(path)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if started.elapsed() < VERSION_CHECK_TIMEOUT => {
                    thread::sleep(Duration::from_millis(20));
                }
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }
}

/// 按平台优先级查找已安装且可运行的 Chromium 系浏览器。
pub struct BrowserLocator {
    os: TargetOs,
    probe: Arc<dyn BrowserProbe>,
}

impl BrowserLocator {
    pub fn new() -> Self {
        Self {
            os: TargetOs::current(),
            probe: Arc::new(SystemBrowserProbe),
        }
    }

    #[cfg(test)]
    fn for_test(os: TargetOs, probe: Arc<dyn BrowserProbe>) -> Self {
        Self { os, probe }
    }

    /// 返回当前平台优先级最高且可用的浏览器。
    pub fn find(&self) -> Result<BrowserExecutable, HtmlRenderError> {
        let candidates = self.candidates();
        let searched = candidates
            .iter()
            .map(|(_, candidate)| candidate.display().to_string())
            .collect::<Vec<_>>();
        for (kind, path) in candidates {
            // Windows 和 macOS 不再仅为查询版本启动浏览器，避免激活用户已有窗口。
            // 后续实际的 headless 启动才是最终可用性检查。
            let available = match self.os {
                TargetOs::Windows | TargetOs::MacOs => self.probe.is_file(&path),
                TargetOs::Linux => self.probe.has_version(&path),
            };
            if available {
                return Ok(BrowserExecutable {
                    kind,
                    path,
                    label: kind.label(),
                });
            }
        }
        Err(HtmlRenderError::RendererUnavailable { searched })
    }

    fn candidates(&self) -> Vec<(BrowserKind, PathBuf)> {
        match self.os {
            TargetOs::Windows => windows_candidates(),
            TargetOs::MacOs => macos_candidates(),
            TargetOs::Linux => linux_candidates(&*self.probe),
        }
    }
}

impl Default for BrowserLocator {
    fn default() -> Self {
        Self::new()
    }
}

fn windows_candidates() -> Vec<(BrowserKind, PathBuf)> {
    let program_files_x86 = env::var_os("PROGRAMFILES(X86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let program_files = env::var_os("PROGRAMFILES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let local_app_data = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let mut candidates = Vec::new();
    for root in [&program_files_x86, &program_files].into_iter() {
        candidates.push((
            BrowserKind::Edge,
            windows_path(root, r"Microsoft\Edge\Application\msedge.exe"),
        ));
    }
    for root in [Some(&program_files), local_app_data.as_ref()]
        .into_iter()
        .flatten()
    {
        candidates.push((
            BrowserKind::Chrome,
            windows_path(root, r"Google\Chrome\Application\chrome.exe"),
        ));
    }
    for root in [Some(&program_files), local_app_data.as_ref()]
        .into_iter()
        .flatten()
    {
        candidates.push((
            BrowserKind::Chromium,
            windows_path(root, r"Chromium\Application\chrome.exe"),
        ));
    }
    candidates
}

fn windows_path(root: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!(r"{}\{suffix}", root.display()))
}

fn macos_candidates() -> Vec<(BrowserKind, PathBuf)> {
    let home_applications =
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Applications"));
    let roots = [Some(PathBuf::from("/Applications")), home_applications];
    let mut candidates = Vec::new();
    for root in roots.iter().flatten() {
        candidates.push((
            BrowserKind::Chrome,
            root.join("Google Chrome.app/Contents/MacOS/Google Chrome"),
        ));
    }
    for root in roots.iter().flatten() {
        candidates.push((
            BrowserKind::Chromium,
            root.join("Chromium.app/Contents/MacOS/Chromium"),
        ));
    }
    candidates
}

fn linux_candidates(probe: &dyn BrowserProbe) -> Vec<(BrowserKind, PathBuf)> {
    [
        (BrowserKind::Chrome, "google-chrome-stable"),
        (BrowserKind::Chrome, "google-chrome"),
        (BrowserKind::Chromium, "chromium"),
        (BrowserKind::Chromium, "chromium-browser"),
    ]
    .into_iter()
    .filter_map(|(kind, command)| probe.find_in_path(command).map(|path| (kind, path)))
    .collect()
}

#[derive(Debug, Clone)]
pub struct BrowserDriverRequest {
    proxy_url: Url,
    target_url: Url,
    profile_path: PathBuf,
    paper: EffectivePaper,
    wait_ms: u64,
    print_background: bool,
    output_path: PathBuf,
    rejected_resources: RejectedResourceTracker,
}

/// 浏览器 worker 与外层 deadline 协作所需的状态。
#[derive(Clone)]
pub struct BrowserRenderControl {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    timeout_ms: u64,
    cdp_operation_timeout: Duration,
}

impl BrowserRenderControl {
    fn new(
        deadline: Instant,
        cancelled: Arc<AtomicBool>,
        timeout_ms: u64,
        cdp_operation_timeout: Duration,
    ) -> Self {
        Self {
            deadline,
            cancelled,
            timeout_ms,
            cdp_operation_timeout,
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn check_active(&self) -> Result<(), HtmlRenderError> {
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            self.cancel();
            return Err(self.timeout_error());
        }
        Ok(())
    }

    fn remaining(&self) -> Result<Duration, HtmlRenderError> {
        self.check_active()?;
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.cancel();
            return Err(self.timeout_error());
        }
        Ok(remaining)
    }

    /// 只在有足够时间完成接下来全部 CDP 请求时允许开始该同步调用。
    fn before_cdp_operations(&self, request_count: u32) -> Result<Duration, HtmlRenderError> {
        let required = self
            .cdp_operation_timeout
            .checked_mul(request_count)
            .expect("fixed CDP operation timeout must not overflow");
        if self.remaining()? < required {
            self.cancel();
            return Err(self.timeout_error());
        }
        Ok(self.cdp_operation_timeout)
    }

    fn wait(&self, duration: Duration) -> Result<(), HtmlRenderError> {
        let wait_deadline = Instant::now()
            .checked_add(duration)
            .unwrap_or(self.deadline);
        loop {
            self.check_active()?;
            let now = Instant::now();
            if now >= wait_deadline {
                return Ok(());
            }
            let remaining_wait = wait_deadline.saturating_duration_since(now);
            let sleep_for = self
                .remaining()?
                .min(remaining_wait)
                .min(WAIT_POLL_INTERVAL);
            thread::sleep(sleep_for);
        }
    }

    fn timeout_error(&self) -> HtmlRenderError {
        HtmlRenderError::Timeout {
            timeout_ms: self.timeout_ms,
        }
    }
}

/// 同步浏览器调用边界，供渲染器放进 blocking 线程并在测试中替换。
pub trait BrowserDriver: Send + Sync {
    fn render(
        &self,
        request: BrowserDriverRequest,
        control: BrowserRenderControl,
    ) -> Result<(), HtmlRenderError>;
}

struct InstalledBrowserDriver {
    locator: BrowserLocator,
}

impl InstalledBrowserDriver {
    fn new() -> Self {
        Self {
            locator: BrowserLocator::new(),
        }
    }
}

impl BrowserDriver for InstalledBrowserDriver {
    fn render(
        &self,
        request: BrowserDriverRequest,
        control: BrowserRenderControl,
    ) -> Result<(), HtmlRenderError> {
        let executable = self.locator.find()?;
        let cdp_operation_timeout = control.before_cdp_operations(1)?;
        let proxy_server = chrome_proxy_server(&request.proxy_url);
        let options = LaunchOptionsBuilder::default()
            .path(Some(executable.path.clone()))
            .user_data_dir(Some(request.profile_path.clone()))
            .proxy_server(Some(&proxy_server))
            .args(vec![std::ffi::OsStr::new(FORCE_LOOPBACK_THROUGH_PROXY)])
            .ignore_certificate_errors(false)
            .headless(true)
            .sandbox(true)
            .idle_browser_timeout(cdp_operation_timeout)
            .build()
            .map_err(HtmlRenderError::browser_options)?;
        let browser =
            Browser::new(options).map_err(|error| HtmlRenderError::RendererUnavailable {
                searched: vec![executable.path.display().to_string(), error.to_string()],
            })?;
        control.before_cdp_operations(1)?;
        let new_tab_wait_timeout = control.remaining()?.saturating_sub(cdp_operation_timeout);
        browser.set_default_timeout(new_tab_wait_timeout);
        let tab = browser.new_tab().map_err(HtmlRenderError::navigation)?;
        control.before_cdp_operations(2)?;
        request.rejected_resources.clear();
        tab.navigate_to(request.target_url.as_str())
            .map_err(HtmlRenderError::navigation)?;
        tab.set_default_timeout(control.remaining()?);
        tab.wait_until_navigated()
            .map_err(HtmlRenderError::navigation)?;
        control.wait(Duration::from_millis(request.wait_ms))?;
        control.before_cdp_operations(2)?;
        let pdf = tab
            .print_to_pdf(Some(PrintToPdfOptions {
                print_background: Some(request.print_background),
                paper_width: Some(request.paper.width_mm / 25.4),
                paper_height: Some(request.paper.height_mm / 25.4),
                margin_top: Some(0.0),
                margin_right: Some(0.0),
                margin_bottom: Some(0.0),
                margin_left: Some(0.0),
                ..Default::default()
            }))
            .map_err(HtmlRenderError::pdf_export)?;
        control.check_active()?;
        std::fs::write(request.output_path, pdf)?;
        Ok(())
    }
}

/// 通过已安装 Chromium 系浏览器输出 PDF 的 HTML 渲染器。
pub struct BrowserHtmlRenderer {
    policy: ResourcePolicy,
    driver: Arc<dyn BrowserDriver>,
    timeout: Duration,
    cdp_operation_timeout: Duration,
}

impl BrowserHtmlRenderer {
    pub fn new(policy: ResourcePolicy) -> Self {
        Self {
            policy,
            driver: Arc::new(InstalledBrowserDriver::new()),
            timeout: RENDER_TIMEOUT,
            cdp_operation_timeout: MAX_CDP_OPERATION_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn for_test(policy: ResourcePolicy, driver: Arc<dyn BrowserDriver>) -> Self {
        Self {
            policy,
            driver,
            timeout: RENDER_TIMEOUT,
            cdp_operation_timeout: MAX_CDP_OPERATION_TIMEOUT,
        }
    }
}

impl HtmlRenderer for BrowserHtmlRenderer {
    fn render(&self, request: HtmlRenderRequest) -> HtmlRenderFuture {
        let policy = self.policy.clone();
        let driver = self.driver.clone();
        let timeout = self.timeout;
        let cdp_operation_timeout = self.cdp_operation_timeout;
        Box::pin(async move {
            let deadline = Instant::now()
                .checked_add(timeout)
                .unwrap_or_else(Instant::now);
            let timeout_ms = timeout.as_millis() as u64;
            let cancelled = Arc::new(AtomicBool::new(false));
            let control = BrowserRenderControl::new(
                deadline,
                cancelled.clone(),
                timeout_ms,
                cdp_operation_timeout,
            );
            let inline_html = match &request.source {
                HtmlSource::Url(_) => None,
                HtmlSource::Inline(html) => Some(html.clone()),
            };
            let mut proxy = FilteringProxy::start(policy, inline_html).await?;
            let target_url = proxy.target_url(request.source.clone());
            let rejected_resources = proxy.rejection_tracker();
            let profile = tempfile::tempdir()?;
            let driver_request = BrowserDriverRequest {
                proxy_url: proxy.proxy_url().clone(),
                target_url,
                profile_path: profile.path().to_path_buf(),
                paper: request.paper,
                wait_ms: request.wait_ms,
                print_background: true,
                output_path: request.output_path.clone(),
                rejected_resources,
            };
            control.before_cdp_operations(1)?;
            let worker_control = control.clone();
            let mut worker =
                tokio::task::spawn_blocking(move || driver.render(driver_request, worker_control));
            match tokio::time::timeout(control.remaining()?, &mut worker).await {
                Ok(Ok(result)) => result?,
                Ok(Err(error)) => {
                    proxy.shutdown().await;
                    return Err(HtmlRenderError::Navigation {
                        message: format!("browser worker stopped unexpectedly: {error}"),
                    });
                }
                Err(_) => {
                    control.cancel();
                    let _ = worker.await;
                    proxy.shutdown().await;
                    return Err(control.timeout_error());
                }
            }
            let rejected_resource = proxy.rejected_resource();
            proxy.shutdown().await;
            if let Some(resource) = rejected_resource {
                return Err(HtmlRenderError::BlockedResource { resource });
            }
            control.check_active()?;
            drop(profile);
            Ok(HtmlRenderResult {
                renderer: "chromium",
                output_path: request.output_path,
            })
        })
    }
}

impl HtmlRenderError {
    fn browser_options(error: impl std::fmt::Display) -> Self {
        Self::Navigation {
            message: format!("browser launch options are invalid: {error}"),
        }
    }

    fn navigation(error: impl std::fmt::Display) -> Self {
        Self::Navigation {
            message: error.to_string(),
        }
    }

    fn pdf_export(error: impl std::fmt::Display) -> Self {
        Self::PdfExport {
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserDriver, BrowserDriverRequest, BrowserHtmlRenderer, BrowserKind, BrowserLocator,
        BrowserProbe, TargetOs,
    };
    use crate::{
        html::{resource_policy::ResourcePolicy, HtmlRenderRequest, HtmlRenderer, HtmlSource},
        protocol::EffectivePaper,
    };
    use std::{
        collections::HashMap,
        io::{ErrorKind, Read, Write},
        net::TcpListener,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };
    use url::Url;

    #[derive(Default)]
    struct FakeProbe {
        files: Vec<PathBuf>,
        commands: HashMap<String, PathBuf>,
    }

    impl BrowserProbe for FakeProbe {
        fn is_file(&self, path: &Path) -> bool {
            self.files.iter().any(|candidate| candidate == path)
        }

        fn find_in_path(&self, command: &str) -> Option<PathBuf> {
            self.commands.get(command).cloned()
        }

        fn has_version(&self, _path: &Path) -> bool {
            true
        }
    }

    fn fake_probe(files: impl IntoIterator<Item = impl AsRef<Path>>) -> Arc<dyn BrowserProbe> {
        Arc::new(FakeProbe {
            files: files
                .into_iter()
                .map(|path| path.as_ref().to_path_buf())
                .collect(),
            ..Default::default()
        })
    }

    fn fake_path_probe(
        commands: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Arc<dyn BrowserProbe> {
        Arc::new(FakeProbe {
            commands: commands
                .into_iter()
                .map(|command| {
                    let command = command.as_ref();
                    (
                        command.to_string(),
                        PathBuf::from("/fake/bin").join(command),
                    )
                })
                .collect(),
            ..Default::default()
        })
    }

    #[test]
    fn windows_prefers_edge_then_chrome_then_chromium() {
        let locator = BrowserLocator::for_test(
            TargetOs::Windows,
            fake_probe([
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            ]),
        );
        assert_eq!(locator.find().unwrap().kind, BrowserKind::Edge);
    }

    #[test]
    fn macos_and_linux_prefer_chrome_then_chromium() {
        let mac = BrowserLocator::for_test(
            TargetOs::MacOs,
            fake_probe([
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
            ]),
        );
        assert_eq!(mac.find().unwrap().kind, BrowserKind::Chrome);

        let linux = BrowserLocator::for_test(
            TargetOs::Linux,
            fake_path_probe(["google-chrome-stable", "chromium"]),
        );
        assert_eq!(linux.find().unwrap().kind, BrowserKind::Chrome);
    }

    #[test]
    fn chrome_proxy_argument_omits_the_url_path_separator() {
        assert_eq!(
            super::chrome_proxy_server(&Url::parse("http://127.0.0.1:43123/").unwrap()),
            "http://127.0.0.1:43123"
        );
    }

    #[derive(Default)]
    struct FakeDriver {
        requests: Mutex<Vec<BrowserDriverRequest>>,
    }

    impl BrowserDriver for FakeDriver {
        fn render(
            &self,
            request: BrowserDriverRequest,
            _control: super::BrowserRenderControl,
        ) -> Result<(), crate::html::HtmlRenderError> {
            assert!(request.profile_path.is_dir());
            self.requests.lock().unwrap().push(request);
            Ok(())
        }
    }

    struct BlockingDriver {
        attempted_write: AtomicBool,
        wrote: AtomicBool,
        saw_profile: AtomicBool,
        profile_path: Mutex<Option<PathBuf>>,
        proxy_url: Mutex<Option<Url>>,
    }

    struct LoopbackRequestDriver {
        blocked_url: String,
    }

    #[derive(Default)]
    struct CountingDriver {
        calls: AtomicUsize,
    }

    impl BrowserDriver for CountingDriver {
        fn render(
            &self,
            _request: BrowserDriverRequest,
            _control: super::BrowserRenderControl,
        ) -> Result<(), crate::html::HtmlRenderError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }
    impl BrowserDriver for BlockingDriver {
        fn render(
            &self,
            request: BrowserDriverRequest,
            control: super::BrowserRenderControl,
        ) -> Result<(), crate::html::HtmlRenderError> {
            self.saw_profile
                .store(request.profile_path.is_dir(), Ordering::Release);
            *self.profile_path.lock().unwrap() = Some(request.profile_path.clone());
            *self.proxy_url.lock().unwrap() = Some(request.proxy_url);
            while !control.cancelled.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            self.attempted_write.store(true, Ordering::Release);
            if control.check_active().is_ok() {
                std::fs::write(&request.output_path, b"late PDF")?;
                self.wrote.store(true, Ordering::Release);
            }
            Ok(())
        }
    }

    impl BrowserDriver for LoopbackRequestDriver {
        fn render(
            &self,
            request: BrowserDriverRequest,
            _control: super::BrowserRenderControl,
        ) -> Result<(), crate::html::HtmlRenderError> {
            let proxy_address = format!(
                "{}:{}",
                request.proxy_url.host_str().unwrap(),
                request.proxy_url.port().unwrap()
            );
            let mut stream = std::net::TcpStream::connect(proxy_address)?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            stream.write_all(
                format!(
                    "GET {} HTTP/1.1\r\nHost: {}\r\nReferer: {}\r\nConnection: close\r\n\r\n",
                    self.blocked_url,
                    self.blocked_url.strip_prefix("http://").unwrap(),
                    request.target_url,
                )
                .as_bytes(),
            )?;
            let mut response = [0_u8; 128];
            let bytes_read = stream.read(&mut response)?;
            assert!(std::str::from_utf8(&response[..bytes_read])
                .unwrap()
                .starts_with("HTTP/1.1 403"));
            Ok(())
        }
    }

    #[tokio::test]
    async fn renderer_passes_proxy_profile_paper_background_and_wait_to_driver() {
        let driver = Arc::new(FakeDriver::default());
        let renderer = BrowserHtmlRenderer::for_test(ResourcePolicy::system(), driver.clone());
        let paper = EffectivePaper {
            width_mm: 100.0,
            height_mm: 150.0,
        };

        for wait_ms in [1_000, 2_400] {
            renderer
                .render(HtmlRenderRequest {
                    source: HtmlSource::Url(
                        Url::parse("https://public.example.com/invoice").unwrap(),
                    ),
                    paper: paper.clone(),
                    wait_ms,
                    output_path: std::env::temp_dir().join(format!("printbridge-{wait_ms}.pdf")),
                })
                .await
                .unwrap();
        }

        let requests = driver.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for (request, wait_ms) in requests.iter().zip([1_000, 2_400]) {
            assert_eq!(
                request.target_url.as_str(),
                "https://public.example.com/invoice"
            );
            assert_eq!(request.proxy_url.scheme(), "http");
            assert_eq!(request.proxy_url.host_str(), Some("127.0.0.1"));
            assert_eq!(request.paper, paper);
            assert!(request.print_background);
            assert_eq!(request.wait_ms, wait_ms);
        }
    }

    #[tokio::test]
    async fn renderer_returns_the_loopback_resource_rejected_by_its_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let blocked_url = format!("http://{}/private.png", listener.local_addr().unwrap());
        let driver = Arc::new(LoopbackRequestDriver {
            blocked_url: blocked_url.clone(),
        });
        let renderer = BrowserHtmlRenderer::for_test(ResourcePolicy::system(), driver);

        let result = renderer
            .render(HtmlRenderRequest {
                source: HtmlSource::Inline(format!("<img src=\"{blocked_url}\" />")),
                paper: EffectivePaper {
                    width_mm: 100.0,
                    height_mm: 150.0,
                },
                wait_ms: 0,
                output_path: std::env::temp_dir().join("printbridge-blocked-resource.pdf"),
            })
            .await;

        assert!(matches!(
            result,
            Err(crate::html::HtmlRenderError::BlockedResource { resource }) if resource == blocked_url
        ));
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    }

    #[tokio::test]
    async fn timeout_cancels_worker_before_late_pdf_write() {
        let driver = Arc::new(BlockingDriver {
            attempted_write: AtomicBool::new(false),
            wrote: AtomicBool::new(false),
            saw_profile: AtomicBool::new(false),
            profile_path: Mutex::new(None),
            proxy_url: Mutex::new(None),
        });
        let mut renderer = BrowserHtmlRenderer::for_test(ResourcePolicy::system(), driver.clone());
        renderer.timeout = Duration::from_millis(10);
        renderer.cdp_operation_timeout = Duration::from_millis(1);
        let output_path = std::env::temp_dir().join("printbridge-timeout.pdf");
        let _ = std::fs::remove_file(&output_path);
        let result = renderer
            .render(HtmlRenderRequest {
                source: HtmlSource::Url(Url::parse("https://public.example.com/").unwrap()),
                paper: EffectivePaper {
                    width_mm: 100.0,
                    height_mm: 150.0,
                },
                wait_ms: 0,
                output_path: output_path.clone(),
            })
            .await;
        assert!(
            matches!(result, Err(crate::html::HtmlRenderError::Timeout { .. })),
            "unexpected result: {result:?}"
        );
        assert!(driver.saw_profile.load(Ordering::Acquire));
        assert!(driver.attempted_write.load(Ordering::Acquire));
        assert!(!driver.wrote.load(Ordering::Acquire));
        assert!(!driver
            .profile_path
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .exists());
        assert!(!output_path.exists());
        let proxy_url = driver.proxy_url.lock().unwrap().clone().unwrap();
        let proxy_port = proxy_url.port().unwrap();
        assert!(matches!(
            tokio::time::timeout(
                Duration::from_millis(100),
                tokio::net::TcpStream::connect(("127.0.0.1", proxy_port)),
            )
            .await,
            Ok(Err(_))
        ));
    }

    #[tokio::test]
    async fn renderer_does_not_start_driver_when_remaining_time_cannot_cover_one_cdp_operation() {
        let driver = Arc::new(CountingDriver::default());
        let mut renderer = BrowserHtmlRenderer::for_test(ResourcePolicy::system(), driver.clone());
        renderer.timeout = Duration::from_millis(1);

        let result = renderer
            .render(HtmlRenderRequest {
                source: HtmlSource::Url(Url::parse("https://public.example.com/").unwrap()),
                paper: EffectivePaper {
                    width_mm: 100.0,
                    height_mm: 150.0,
                },
                wait_ms: 0,
                output_path: std::env::temp_dir().join("printbridge-capacity.pdf"),
            })
            .await;

        assert!(
            matches!(result, Err(crate::html::HtmlRenderError::Timeout { .. })),
            "unexpected result: {result:?}"
        );
        assert_eq!(driver.calls.load(Ordering::Acquire), 0);
    }

    #[test]
    fn wait_stops_when_deadline_arrives() {
        let control = super::BrowserRenderControl::new(
            Instant::now() + Duration::from_millis(1),
            Arc::new(AtomicBool::new(false)),
            1,
            Duration::from_millis(1),
        );

        let result = control.wait(Duration::from_secs(1));

        assert!(matches!(
            result,
            Err(crate::html::HtmlRenderError::Timeout { .. })
        ));
    }

    #[test]
    fn wait_stops_when_outer_renderer_cancels() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let control = super::BrowserRenderControl::new(
            Instant::now() + Duration::from_secs(1),
            cancelled.clone(),
            1_000,
            Duration::from_millis(1),
        );
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(1));
            cancelled.store(true, Ordering::Release);
        });

        let result = control.wait(Duration::from_secs(1));
        canceller.join().unwrap();

        assert!(matches!(
            result,
            Err(crate::html::HtmlRenderError::Timeout { .. })
        ));
    }
}
