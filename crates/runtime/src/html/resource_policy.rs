#[cfg(test)]
mod tests {
    use super::{HostResolver, ResourcePolicy};
    use crate::html::HtmlRenderError;
    use std::{
        collections::HashMap,
        future::Future,
        io,
        net::{IpAddr, SocketAddr},
        pin::Pin,
        sync::Arc,
    };
    use url::Url;

    struct FakeResolver {
        addresses: HashMap<String, Vec<IpAddr>>,
    }

    impl FakeResolver {
        fn new(entries: impl IntoIterator<Item = (&'static str, Vec<IpAddr>)>) -> Self {
            Self {
                addresses: entries
                    .into_iter()
                    .map(|(host, addresses)| (host.to_string(), addresses))
                    .collect(),
            }
        }
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

    fn policy() -> ResourcePolicy {
        ResourcePolicy::new(Arc::new(FakeResolver::new([
            ("cdn.example.com", vec!["93.184.216.34".parse().unwrap()]),
            ("rebound.example.com", vec!["10.0.0.8".parse().unwrap()]),
            (
                "mixed.example.com",
                vec![
                    "93.184.216.34".parse().unwrap(),
                    "10.0.0.8".parse().unwrap(),
                ],
            ),
        ])))
    }

    #[tokio::test]
    async fn policy_allows_public_http_and_https_only() {
        let policy = policy();

        assert!(policy
            .resolve_public_target(&Url::parse("https://cdn.example.com/a.css").unwrap())
            .await
            .is_ok());
        assert!(matches!(
            policy
                .resolve_public_target(&Url::parse("https://rebound.example.com/a.css").unwrap())
                .await,
            Err(HtmlRenderError::BlockedResource { .. })
        ));
    }

    #[tokio::test]
    async fn policy_rejects_non_public_schemes_and_hosts() {
        let policy = policy();
        for value in [
            "file:///tmp/secret.html",
            "data:text/html,secret",
            "http://localhost/",
            "http://localhost./",
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://10.0.0.8/",
            "http://169.254.1.1/",
            "http://0.0.0.0/",
            "http://224.0.0.1/",
            "http://[::ffff:127.0.0.1]/",
            "http://[fec0::1]/",
        ] {
            assert!(matches!(
                policy
                    .resolve_public_target(&Url::parse(value).unwrap())
                    .await,
                Err(HtmlRenderError::BlockedResource { .. })
            ));
        }
    }

    #[tokio::test]
    async fn policy_rejects_empty_and_mixed_dns_results() {
        let policy = policy();
        for value in ["https://unknown.example.com/", "https://mixed.example.com/"] {
            assert!(matches!(
                policy
                    .resolve_public_target(&Url::parse(value).unwrap())
                    .await,
                Err(HtmlRenderError::BlockedResource { .. })
            ));
        }
    }
}
use crate::html::HtmlRenderError;
use ipnet::IpNet;
use std::{
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
};
use url::Url;

/// 将主机名解析为待连接地址的接口。
pub trait HostResolver: Send + Sync {
    /// 解析主机名及端口，返回所有候选地址。
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>>;
}

/// 使用系统 DNS 解析主机名。
pub struct SystemResolver;

impl HostResolver for SystemResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>> {
        Box::pin(async move { Ok(tokio::net::lookup_host((host, port)).await?.collect()) })
    }
}

/// 经过资源策略验证后可直接连接的目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    /// 原始目标的 authority，供 HTTP Host 与 CONNECT 使用。
    pub authority: String,
    /// 已通过验证、必须用于实际连接的地址。
    pub address: SocketAddr,
}

/// 只允许 HTML 渲染访问公开 HTTP(S) 资源的策略。
#[derive(Clone)]
pub struct ResourcePolicy {
    resolver: Arc<dyn HostResolver>,
}

impl ResourcePolicy {
    /// 使用可注入 DNS 解析器创建策略。
    pub fn new(resolver: Arc<dyn HostResolver>) -> Self {
        Self { resolver }
    }

    /// 使用操作系统 DNS 解析器创建策略。
    pub fn system() -> Self {
        Self::new(Arc::new(SystemResolver))
    }

    /// 验证 URL、解析一次，并返回已经批准的连接地址。
    pub async fn resolve_public_target(
        &self,
        url: &Url,
    ) -> Result<ResolvedTarget, HtmlRenderError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(blocked(url));
        }

        let host = url.host_str().ok_or_else(|| blocked(url))?;
        let normalized_host = host.trim_end_matches('.').to_ascii_lowercase();
        if normalized_host.is_empty() || normalized_host == "localhost" {
            return Err(blocked(url));
        }
        let port = url.port_or_known_default().ok_or_else(|| blocked(url))?;
        let authority = authority(url, host);

        if let Ok(address) = normalized_host.parse::<IpAddr>() {
            return approved_target(url, authority, SocketAddr::new(address, port));
        }

        let addresses = self
            .resolver
            .resolve(&normalized_host, port)
            .await
            .map_err(|_| blocked(url))?;
        let Some(address) = addresses.first().copied() else {
            return Err(blocked(url));
        };
        if addresses
            .iter()
            .any(|candidate| !is_public_ip(candidate.ip()))
        {
            return Err(blocked(url));
        }

        approved_target(url, authority, address)
    }
}

fn authority(url: &Url, host: &str) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn approved_target(
    url: &Url,
    authority: String,
    address: SocketAddr,
) -> Result<ResolvedTarget, HtmlRenderError> {
    if is_public_ip(address.ip()) {
        Ok(ResolvedTarget { authority, address })
    } else {
        Err(blocked(url))
    }
}

fn blocked(url: &Url) -> HtmlRenderError {
    HtmlRenderError::BlockedResource {
        resource: url.as_str().to_string(),
    }
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => !matches_ip_net(IpAddr::V4(address), BLOCKED_IPV4_NETS),
        IpAddr::V6(address) => {
            address.to_ipv4_mapped().is_none()
                && !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_unicast_link_local()
                && !matches_ip_net(IpAddr::V6(address), BLOCKED_IPV6_NETS)
        }
    }
}

fn matches_ip_net(address: IpAddr, networks: &[&str]) -> bool {
    networks.iter().any(|network| {
        network
            .parse::<IpNet>()
            .is_ok_and(|network| network.contains(&address))
    })
}

const BLOCKED_IPV4_NETS: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.88.99.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/4",
    "240.0.0.0/4",
];

const BLOCKED_IPV6_NETS: &[&str] = &["100::/64", "2001:db8::/32", "fc00::/7", "fec0::/10"];
