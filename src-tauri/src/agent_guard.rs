use crate::config::AgentConfig;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAgent {
    pub addr: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPortStatus {
    Available,
    PrintBridge(RunningAgent),
    OccupiedByOther { addr: SocketAddr },
}

/// 检查当前配置端口上是否已有 PrintBridge Agent。
pub fn check_agent_port(config: &AgentConfig) -> AgentPortStatus {
    let addr = probe_addr(config);
    match TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) {
        Ok(mut stream) => probe_connected_stream(addr, &mut stream),
        Err(_) => AgentPortStatus::Available,
    }
}

/// 返回发现已有 Agent 时展示给 CLI 或 GUI 的消息。
pub fn already_running_message(agent: &RunningAgent) -> String {
    format!("PrintBridge Agent is already running at {}", agent.addr)
}

fn probe_addr(config: &AgentConfig) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], config.service.port))
}

fn probe_connected_stream(addr: SocketAddr, stream: &mut TcpStream) -> AgentPortStatus {
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let request = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";

    if stream.write_all(request).is_err() {
        return AgentPortStatus::OccupiedByOther { addr };
    }

    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return AgentPortStatus::OccupiedByOther { addr };
    }

    if is_print_bridge_health_response(&response) {
        AgentPortStatus::PrintBridge(RunningAgent { addr })
    } else {
        AgentPortStatus::OccupiedByOther { addr }
    }
}

fn is_print_bridge_health_response(response: &str) -> bool {
    let Some((head, body)) = response.split_once("\r\n\r\n") else {
        return false;
    };
    let Some(status_line) = head.lines().next() else {
        return false;
    };
    if !status_line.contains(" 200 ") {
        return false;
    }

    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("service")
                .and_then(|service| service.as_str())
                .map(str::to_string)
        })
        .is_some_and(|service| service == "print-bridge")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn config_for_port(port: u16) -> AgentConfig {
        let mut config = AgentConfig::default();
        config.service.port = port;
        config
    }

    fn start_one_response_server(response: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 512];
            let _ = stream.read(&mut request);
            stream.write_all(response.as_bytes()).unwrap();
        });
        port
    }

    #[test]
    fn check_agent_port_reports_available_when_nothing_listens() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        assert_eq!(
            check_agent_port(&config_for_port(port)),
            AgentPortStatus::Available
        );
    }

    #[test]
    fn check_agent_port_detects_print_bridge_health_response() {
        let port = start_one_response_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"ok\",\"service\":\"print-bridge\"}",
        );

        assert_eq!(
            check_agent_port(&config_for_port(port)),
            AgentPortStatus::PrintBridge(RunningAgent {
                addr: SocketAddr::from(([127, 0, 0, 1], port))
            })
        );
    }

    #[test]
    fn check_agent_port_does_not_treat_other_service_as_print_bridge() {
        let port = start_one_response_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"ok\",\"service\":\"other\"}",
        );

        assert_eq!(
            check_agent_port(&config_for_port(port)),
            AgentPortStatus::OccupiedByOther {
                addr: SocketAddr::from(([127, 0, 0, 1], port))
            }
        );
    }

    #[test]
    fn already_running_message_names_addr() {
        let agent = RunningAgent {
            addr: "127.0.0.1:17890".parse().unwrap(),
        };

        assert_eq!(
            already_running_message(&agent),
            "PrintBridge Agent is already running at 127.0.0.1:17890"
        );
    }
}
