use print_bridge_core::{
    config::AgentConfig,
    protocol::{ClientMessage, ServerMessage},
};

#[test]
fn core_exports_protocol_and_config_types() {
    let _: AgentConfig = AgentConfig::default();
    let message: ClientMessage = serde_json::from_str(r#"{"type":"ping","time":1}"#).unwrap();
    assert!(matches!(message, ClientMessage::Ping { time: 1 }));

    let response = ServerMessage::Pong {
        time: 1,
        agent_status: "ready".to_string(),
    };
    assert_eq!(serde_json::to_value(response).unwrap()["type"], "pong");
}
