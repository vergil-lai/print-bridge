use crate::config::AgentConfig;
use std::{
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
        Ok(_) => AgentPortStatus::OccupiedByOther { addr },
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn config_for_port(port: u16) -> AgentConfig {
        let mut config = AgentConfig::default();
        config.service.port = port;
        config
    }

    fn start_server() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
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
    fn check_agent_port_reports_any_listener_as_occupied() {
        let (_listener, port) = start_server();

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
