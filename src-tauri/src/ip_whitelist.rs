use ipnet::IpNet;
use std::net::IpAddr;

/// IP 白名单中始终保留的本机回环地址。
pub const REQUIRED_LOOPBACK_IP: &str = "127.0.0.1";

/// 规范化 IP 白名单，并确保本机回环地址始终存在。
pub fn normalize_allowed_ips(entries: Vec<String>) -> Vec<String> {
    let mut normalized = vec![REQUIRED_LOOPBACK_IP.to_string()];

    for entry in entries {
        let entry = entry.trim();
        if entry.is_empty() || entry == REQUIRED_LOOPBACK_IP {
            continue;
        }
        if !normalized.iter().any(|item| item == entry) {
            normalized.push(entry.to_string());
        }
    }

    normalized
}

/// 校验单个 IP 白名单项是否为合法 IP 或 CIDR 网段。
pub fn validate_allowed_ip_entry(entry: &str) -> Result<(), String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return Err("IP 白名单不能为空".to_string());
    }

    if entry == "0.0.0.0" || entry == "::" || entry == "0.0.0.0/0" || entry == "::/0" {
        return Err("不能使用任意地址作为 IP 白名单项".to_string());
    }

    if entry.contains('/') {
        let net = entry
            .parse::<IpNet>()
            .map_err(|_| format!("IP 或网段无效: {entry}"))?;
        if net.prefix_len() == 0 {
            return Err("不能使用任意地址作为 IP 白名单项".to_string());
        }
        return Ok(());
    }

    entry
        .parse::<IpAddr>()
        .map(drop)
        .map_err(|_| format!("IP 或网段无效: {entry}"))
}

/// 判断客户端 IP 是否命中白名单中的单个 IP 或 CIDR 网段。
pub fn is_client_ip_allowed(client_ip: IpAddr, entries: &[String]) -> bool {
    if client_ip.is_loopback() {
        return true;
    }

    entries
        .iter()
        .any(|entry| entry_matches_client_ip(entry, client_ip))
}

/// 判断单条白名单配置是否匹配客户端 IP。
fn entry_matches_client_ip(entry: &str, client_ip: IpAddr) -> bool {
    if let Ok(ip) = entry.parse::<IpAddr>() {
        return ip == client_ip;
    }

    entry
        .parse::<IpNet>()
        .is_ok_and(|network| network.contains(&client_ip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn normalize_adds_loopback_trims_and_deduplicates() {
        let entries = normalize_allowed_ips(vec![
            " 192.168.1.0/24 ".to_string(),
            "127.0.0.1".to_string(),
            "192.168.1.0/24".to_string(),
        ]);

        assert_eq!(entries, vec!["127.0.0.1", "192.168.1.0/24"]);
    }

    #[test]
    fn validate_accepts_single_ip_and_cidr() {
        assert!(validate_allowed_ip_entry("192.168.1.23").is_ok());
        assert!(validate_allowed_ip_entry("192.168.1.0/24").is_ok());
        assert!(validate_allowed_ip_entry("::1").is_ok());
        assert!(validate_allowed_ip_entry("fd00::/8").is_ok());
    }

    #[test]
    fn validate_rejects_invalid_and_any_address_entries() {
        for entry in [
            "",
            "not-an-ip",
            "192.168.1.0/33",
            "fd00::/129",
            "0.0.0.0",
            "::",
            "0.0.0.0/0",
            "::/0",
        ] {
            assert!(
                validate_allowed_ip_entry(entry).is_err(),
                "{entry} should be rejected"
            );
        }
    }

    #[test]
    fn client_ip_matches_single_ip_and_cidr() {
        let entries = vec![
            "127.0.0.1".to_string(),
            "192.168.1.0/24".to_string(),
            "10.0.0.8".to_string(),
        ];

        assert!(is_client_ip_allowed(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            &entries
        ));
        assert!(is_client_ip_allowed(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            &entries
        ));
        assert!(is_client_ip_allowed(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)),
            &entries
        ));
        assert!(!is_client_ip_allowed(
            IpAddr::V4(Ipv4Addr::new(192, 168, 2, 20)),
            &entries
        ));
    }

    #[test]
    fn client_ip_matches_ipv6_cidr() {
        let entries = vec!["fd00::/8".to_string()];

        assert!(is_client_ip_allowed(
            IpAddr::V6("fd00::1234".parse().unwrap()),
            &entries
        ));
        assert!(!is_client_ip_allowed(
            IpAddr::V6("fe80::1234".parse().unwrap()),
            &entries
        ));
    }
}
