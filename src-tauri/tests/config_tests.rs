use print_bridge_lib::{
    config::{AgentConfig, AppConfig, LimitsConfig, PrintingConfig, SecurityConfig, ServiceConfig},
    protocol::EffectivePaper,
};
use std::fs;

#[test]
fn agent_config_defaults_match_grouped_baseline() {
    let config = AgentConfig::default();

    assert_eq!(config.service.host, "127.0.0.1");
    assert_eq!(config.service.port, 17890);
    assert!(config.security.allowed_origins.is_empty());
    assert_eq!(config.printing.default_printer, None);
    assert_eq!(config.printing.default_paper, None);
    assert_eq!(config.printing.default_copies, 1);
    assert_eq!(config.limits.max_file_size_mb, 20);
    assert_eq!(config.limits.max_batch_jobs, 20);
    assert_eq!(config.limits.max_copies, 100);
    assert_eq!(config.limits.download_timeout_seconds, 30);
    assert!(!config.app.autostart);
}

#[test]
fn agent_config_json_roundtrips() {
    let config = AgentConfig {
        service: ServiceConfig {
            host: "0.0.0.0".to_string(),
            port: 19000,
        },
        security: SecurityConfig {
            allowed_origins: vec!["http://localhost:5173".to_string()],
        },
        printing: PrintingConfig {
            default_printer: Some("TSC TE244".to_string()),
            default_paper: Some(EffectivePaper {
                width_mm: 60.0,
                height_mm: 40.0,
            }),
            default_copies: 2,
        },
        limits: LimitsConfig {
            max_file_size_mb: 10,
            max_batch_jobs: 3,
            max_copies: 8,
            download_timeout_seconds: 15,
        },
        app: AppConfig { autostart: true },
    };

    let json = serde_json::to_string(&config).unwrap();
    let decoded: AgentConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, config);
}

#[test]
fn agent_config_load_returns_default_when_file_is_missing() {
    let path = std::env::temp_dir().join(format!(
        "print-bridge-missing-config-{}.json",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);

    let config = AgentConfig::load(&path).unwrap();

    assert_eq!(config, AgentConfig::default());
}

#[test]
fn agent_config_save_and_load_roundtrips() {
    let path = std::env::temp_dir().join(format!(
        "print-bridge-config-roundtrip-{}.json",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = AgentConfig::default();

    config.save(&path).unwrap();
    let loaded = AgentConfig::load(&path).unwrap();

    assert_eq!(loaded, config);

    let _ = fs::remove_file(&path);
}

#[test]
fn agent_config_load_returns_error_for_invalid_json() {
    let path = std::env::temp_dir().join(format!(
        "print-bridge-invalid-config-{}.json",
        std::process::id()
    ));
    fs::write(&path, "{ invalid json").unwrap();

    let result = AgentConfig::load(&path);

    assert!(result.is_err());

    let _ = fs::remove_file(&path);
}
