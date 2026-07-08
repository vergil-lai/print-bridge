use crate::config::AgentConfig;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};
use thiserror::Error;
use url::Url;
use zeroize::Zeroize;

const ENCRYPTED_FORMAT: &str = "printbridge-config-encrypted";
const PAYLOAD_FORMAT: &str = "printbridge-config";
const TRANSFER_VERSION: u16 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const ARGON2_MEMORY_KIB: u32 = 19_456;
const ARGON2_ITERATIONS: u32 = 2;
const ARGON2_PARALLELISM: u32 = 1;
const GCM_TAG_BYTES: u8 = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportConfigOptions {
    pub service_port: bool,
    pub allowed_origins: bool,
    pub remote_enabled: bool,
    pub remote_endpoint_url: bool,
    pub remote_bearer_token: bool,
    pub remote_poll_interval_seconds: bool,
    pub remote_max_report_retries: bool,
}

impl ExportConfigOptions {
    pub fn all() -> Self {
        Self {
            service_port: true,
            allowed_origins: true,
            remote_enabled: true,
            remote_endpoint_url: true,
            remote_bearer_token: true,
            remote_poll_interval_seconds: true,
            remote_max_report_retries: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedConfigFile {
    pub format: String,
    pub version: u16,
    pub crypto: CryptoMetadata,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CryptoMetadata {
    pub kdf: String,
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    pub cipher: String,
    pub tag_bytes: u8,
    pub salt: String,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigTransferPayload {
    pub format: String,
    pub version: u16,
    pub config: PartialTransferConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartialTransferConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<PartialServiceTransferConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<PartialSecurityTransferConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<PartialRemoteTransferConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartialServiceTransferConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartialSecurityTransferConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_origins: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartialRemoteTransferConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub endpoint_url: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub bearer_token: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_report_retries: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportPreview {
    pub file_hash: String,
    pub items: Vec<ImportPreviewItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportPreviewItem {
    pub key: String,
    pub label: String,
    pub current: String,
    pub next: String,
}

#[derive(Debug, Error)]
pub enum ConfigTransferError {
    #[error("不是有效的 PrintBridge 配置文件")]
    InvalidFile,
    #[error("密码错误或文件损坏")]
    InvalidPasswordOrPayload,
    #[error("配置导出失败")]
    Serialize,
    #[error("{0}")]
    InvalidField(String),
}

impl From<io::Error> for ConfigTransferError {
    fn from(error: io::Error) -> Self {
        ConfigTransferError::InvalidField(error.to_string())
    }
}

pub fn write_encrypted_file(
    path: &Path,
    file: &EncryptedConfigFile,
) -> Result<(), ConfigTransferError> {
    let content = serde_json::to_string_pretty(file).map_err(|_| ConfigTransferError::Serialize)?;
    fs::write(path, content)?;
    Ok(())
}

pub fn read_encrypted_file(path: &Path) -> Result<EncryptedConfigFile, ConfigTransferError> {
    let (file, _) = read_encrypted_file_with_hash(path)?;
    Ok(file)
}

pub fn read_encrypted_file_with_hash(
    path: &Path,
) -> Result<(EncryptedConfigFile, String), ConfigTransferError> {
    let content = fs::read_to_string(path).map_err(|_| ConfigTransferError::InvalidFile)?;
    let hash = sha256_hex(content.as_bytes());
    let file = serde_json::from_str::<EncryptedConfigFile>(&content)
        .map_err(|_| ConfigTransferError::InvalidFile)?;
    Ok((file, hash))
}

fn sha256_hex(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String should not fail");
    }
    out
}

mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        match value {
            Some(Some(inner)) => serializer.serialize_some(inner),
            Some(None) | None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

pub fn build_transfer_payload(
    config: &AgentConfig,
    options: &ExportConfigOptions,
) -> ConfigTransferPayload {
    let service = options.service_port.then(|| PartialServiceTransferConfig {
        port: Some(config.service.port),
    });

    let security = options
        .allowed_origins
        .then(|| PartialSecurityTransferConfig {
            allowed_origins: Some(config.security.allowed_origins.clone()),
        });

    let remote = build_remote_transfer(config, options);

    ConfigTransferPayload {
        format: PAYLOAD_FORMAT.to_string(),
        version: TRANSFER_VERSION,
        config: PartialTransferConfig {
            service,
            security,
            remote,
        },
    }
}

fn build_remote_transfer(
    config: &AgentConfig,
    options: &ExportConfigOptions,
) -> Option<PartialRemoteTransferConfig> {
    let remote = PartialRemoteTransferConfig {
        enabled: options.remote_enabled.then_some(config.remote.enabled),
        endpoint_url: options
            .remote_endpoint_url
            .then(|| config.remote.endpoint_url.clone()),
        bearer_token: options
            .remote_bearer_token
            .then(|| config.remote.bearer_token.clone()),
        poll_interval_seconds: options
            .remote_poll_interval_seconds
            .then_some(config.remote.poll_interval_seconds),
        max_report_retries: options
            .remote_max_report_retries
            .then_some(config.remote.max_report_retries),
    };

    if remote.enabled.is_some()
        || remote.endpoint_url.is_some()
        || remote.bearer_token.is_some()
        || remote.poll_interval_seconds.is_some()
        || remote.max_report_retries.is_some()
    {
        Some(remote)
    } else {
        None
    }
}

pub fn encrypt_payload(
    payload: &ConfigTransferPayload,
    password: &str,
) -> Result<EncryptedConfigFile, ConfigTransferError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let mut key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| ConfigTransferError::Serialize)?;
    let plaintext = serde_json::to_vec(payload).map_err(|_| ConfigTransferError::Serialize)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| ConfigTransferError::Serialize)?;
    key.zeroize();

    Ok(EncryptedConfigFile {
        format: ENCRYPTED_FORMAT.to_string(),
        version: TRANSFER_VERSION,
        crypto: CryptoMetadata {
            kdf: "argon2id13".to_string(),
            memory_kib: ARGON2_MEMORY_KIB,
            iterations: ARGON2_ITERATIONS,
            parallelism: ARGON2_PARALLELISM,
            cipher: "aes-256-gcm".to_string(),
            tag_bytes: GCM_TAG_BYTES,
            salt: BASE64.encode(salt),
            nonce: BASE64.encode(nonce),
        },
        payload: BASE64.encode(ciphertext),
    })
}

pub fn decrypt_payload(
    file: &EncryptedConfigFile,
    password: &str,
) -> Result<ConfigTransferPayload, ConfigTransferError> {
    validate_envelope(file)?;
    let salt = decode_fixed::<SALT_LEN>(&file.crypto.salt)?;
    let nonce = decode_fixed::<NONCE_LEN>(&file.crypto.nonce)?;
    let ciphertext = BASE64
        .decode(&file.payload)
        .map_err(|_| ConfigTransferError::InvalidPasswordOrPayload)?;

    let mut key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| ConfigTransferError::InvalidPasswordOrPayload)?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| ConfigTransferError::InvalidPasswordOrPayload)?;
    key.zeroize();

    let payload = serde_json::from_slice::<ConfigTransferPayload>(&plaintext)
        .map_err(|_| ConfigTransferError::InvalidPasswordOrPayload)?;
    validate_payload(&payload)?;
    Ok(payload)
}

pub fn merge_payload(
    current: &AgentConfig,
    payload: &ConfigTransferPayload,
) -> Result<AgentConfig, ConfigTransferError> {
    validate_payload(payload)?;
    let mut next = current.clone();

    if let Some(service) = &payload.config.service {
        if let Some(port) = service.port {
            if port == 0 {
                return Err(ConfigTransferError::InvalidField(
                    "本地端口必须大于 0".to_string(),
                ));
            }
            next.service.port = port;
        }
    }

    if let Some(security) = &payload.config.security {
        if let Some(allowed_origins) = &security.allowed_origins {
            for origin in allowed_origins {
                crate::protocol::validate_origin(origin).map_err(|_| {
                    ConfigTransferError::InvalidField(format!("Origin 无效: {origin}"))
                })?;
            }
            next.security.allowed_origins = allowed_origins.clone();
        }
    }

    if let Some(remote) = &payload.config.remote {
        if let Some(enabled) = remote.enabled {
            next.remote.enabled = enabled;
        }

        if let Some(endpoint_url) = &remote.endpoint_url {
            next.remote.endpoint_url = normalize_endpoint_url(endpoint_url.as_deref())?;
        }

        if let Some(bearer_token) = &remote.bearer_token {
            if let Some(token) = bearer_token
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                next.remote.bearer_token = Some(token.to_string());
            }
        }

        if let Some(seconds) = remote.poll_interval_seconds {
            if seconds == 0 {
                return Err(ConfigTransferError::InvalidField(
                    "轮询时间必须大于等于 1 秒".to_string(),
                ));
            }
            next.remote.poll_interval_seconds = seconds;
        }

        if let Some(retries) = remote.max_report_retries {
            if retries == 0 {
                return Err(ConfigTransferError::InvalidField(
                    "上报重试次数必须大于等于 1".to_string(),
                ));
            }
            next.remote.max_report_retries = retries;
        }
    }

    Ok(next.normalized())
}

pub fn preview_payload(
    current: &AgentConfig,
    payload: &ConfigTransferPayload,
) -> Result<ImportPreview, ConfigTransferError> {
    let next = merge_payload(current, payload)?;
    let mut items = Vec::new();

    if payload
        .config
        .service
        .as_ref()
        .is_some_and(|service| service.port.is_some())
    {
        items.push(preview_item(
            "service.port",
            "本地端口",
            current.service.port.to_string(),
            next.service.port.to_string(),
        ));
    }

    if payload
        .config
        .security
        .as_ref()
        .is_some_and(|security| security.allowed_origins.is_some())
    {
        items.push(preview_item(
            "security.allowed_origins",
            "Origin 白名单列表",
            format!("{} 项", current.security.allowed_origins.len()),
            format!("{} 项", next.security.allowed_origins.len()),
        ));
    }

    if let Some(remote) = &payload.config.remote {
        if remote.enabled.is_some() {
            items.push(preview_item(
                "remote.enabled",
                "远程任务开关",
                display_bool(current.remote.enabled),
                display_bool(next.remote.enabled),
            ));
        }

        if remote.endpoint_url.is_some() {
            items.push(preview_item(
                "remote.endpoint_url",
                "远程任务 URL",
                display_optional_string(current.remote.endpoint_url.as_deref()),
                display_optional_string(next.remote.endpoint_url.as_deref()),
            ));
        }

        if remote.bearer_token.is_some() {
            items.push(preview_item(
                "remote.bearer_token",
                "远程任务 Authorization Token",
                display_token_current(current.remote.bearer_token.as_deref()),
                display_token_next(remote.bearer_token.as_ref()),
            ));
        }

        if remote.poll_interval_seconds.is_some() {
            items.push(preview_item(
                "remote.poll_interval_seconds",
                "轮询时间",
                display_seconds(current.remote.poll_interval_seconds),
                display_seconds(next.remote.poll_interval_seconds),
            ));
        }

        if remote.max_report_retries.is_some() {
            items.push(preview_item(
                "remote.max_report_retries",
                "上报重试次数",
                current.remote.max_report_retries.to_string(),
                next.remote.max_report_retries.to_string(),
            ));
        }
    }

    Ok(ImportPreview {
        file_hash: String::new(),
        items,
    })
}

fn normalize_endpoint_url(value: Option<&str>) -> Result<Option<String>, ConfigTransferError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let url = Url::parse(value)
        .map_err(|_| ConfigTransferError::InvalidField("远程任务 URL 无效".to_string()))?;

    match url.scheme() {
        "http" | "https" => Ok(Some(value.to_string())),
        _ => Err(ConfigTransferError::InvalidField(
            "远程任务 URL 只支持 http 或 https".to_string(),
        )),
    }
}

fn preview_item(
    key: &str,
    label: &str,
    current: impl Into<String>,
    next: impl Into<String>,
) -> ImportPreviewItem {
    ImportPreviewItem {
        key: key.to_string(),
        label: label.to_string(),
        current: current.into(),
        next: next.into(),
    }
}

fn display_bool(value: bool) -> String {
    if value {
        "开启".to_string()
    } else {
        "关闭".to_string()
    }
}

fn display_optional_string(value: Option<&str>) -> String {
    value.unwrap_or("未设置").to_string()
}

fn display_token_current(value: Option<&str>) -> String {
    if value.is_some_and(|token| !token.trim().is_empty()) {
        "已设置".to_string()
    } else {
        "未设置".to_string()
    }
}

fn display_token_next(value: Option<&Option<String>>) -> String {
    if value
        .and_then(|token| token.as_deref())
        .is_some_and(|token| !token.trim().is_empty())
    {
        "将覆盖".to_string()
    } else {
        "保留当前".to_string()
    }
}

fn display_seconds(value: u64) -> String {
    format!("{value} 秒")
}

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], ConfigTransferError> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        None,
    )
    .map_err(|_| ConfigTransferError::Serialize)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| ConfigTransferError::InvalidPasswordOrPayload)?;
    Ok(key)
}

fn decode_fixed<const N: usize>(value: &str) -> Result<[u8; N], ConfigTransferError> {
    let bytes = BASE64
        .decode(value)
        .map_err(|_| ConfigTransferError::InvalidPasswordOrPayload)?;
    bytes
        .try_into()
        .map_err(|_| ConfigTransferError::InvalidPasswordOrPayload)
}

fn validate_envelope(file: &EncryptedConfigFile) -> Result<(), ConfigTransferError> {
    if file.format != ENCRYPTED_FORMAT
        || file.version != TRANSFER_VERSION
        || file.crypto.kdf != "argon2id13"
        || file.crypto.memory_kib != ARGON2_MEMORY_KIB
        || file.crypto.iterations != ARGON2_ITERATIONS
        || file.crypto.parallelism != ARGON2_PARALLELISM
        || file.crypto.cipher != "aes-256-gcm"
        || file.crypto.tag_bytes != GCM_TAG_BYTES
    {
        return Err(ConfigTransferError::InvalidFile);
    }
    Ok(())
}

fn validate_payload(payload: &ConfigTransferPayload) -> Result<(), ConfigTransferError> {
    if payload.format != PAYLOAD_FORMAT || payload.version != TRANSFER_VERSION {
        return Err(ConfigTransferError::InvalidFile);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, RemoteConfig};

    fn sample_config() -> AgentConfig {
        let mut config = AgentConfig::default();
        config.service.port = 19090;
        config.security.allowed_origins = vec!["https://example.com".to_string()];
        config.remote = RemoteConfig {
            enabled: true,
            endpoint_url: Some("https://api.example.com/tasks".to_string()),
            bearer_token: Some("secret-token".to_string()),
            device_id: Some("device-1".to_string()),
            device_name: Some("Front Desk".to_string()),
            poll_interval_seconds: 15,
            max_report_retries: 5,
            history_retention_days: 3,
        };
        config
    }

    #[test]
    fn build_payload_only_includes_selected_fields() {
        let config = sample_config();
        let options = ExportConfigOptions {
            service_port: true,
            allowed_origins: false,
            remote_enabled: true,
            remote_endpoint_url: false,
            remote_bearer_token: false,
            remote_poll_interval_seconds: true,
            remote_max_report_retries: false,
        };

        let payload = build_transfer_payload(&config, &options);

        assert_eq!(payload.format, "printbridge-config");
        assert_eq!(payload.version, 1);
        assert_eq!(payload.config.service.unwrap().port, Some(19090));
        assert!(payload.config.security.is_none());
        let remote = payload.config.remote.unwrap();
        assert_eq!(remote.enabled, Some(true));
        assert_eq!(remote.poll_interval_seconds, Some(15));
        assert_eq!(remote.endpoint_url, None);
        assert_eq!(remote.bearer_token, None);
        assert_eq!(remote.max_report_retries, None);
    }

    #[test]
    fn encrypted_payload_roundtrips_with_non_empty_password() {
        let payload = build_transfer_payload(&sample_config(), &ExportConfigOptions::all());

        let encrypted = encrypt_payload(&payload, "passw0rd").unwrap();
        let decrypted = decrypt_payload(&encrypted, "passw0rd").unwrap();

        assert_eq!(decrypted, payload);
        assert_eq!(encrypted.format, "printbridge-config-encrypted");
        assert_eq!(encrypted.version, 1);
        assert_eq!(encrypted.crypto.kdf, "argon2id13");
        assert_eq!(encrypted.crypto.memory_kib, 19_456);
        assert_eq!(encrypted.crypto.iterations, 2);
        assert_eq!(encrypted.crypto.parallelism, 1);
        assert_eq!(encrypted.crypto.cipher, "aes-256-gcm");
        assert_eq!(encrypted.crypto.tag_bytes, 16);
    }

    #[test]
    fn encrypted_payload_roundtrips_with_empty_password() {
        let payload = build_transfer_payload(&sample_config(), &ExportConfigOptions::all());

        let encrypted = encrypt_payload(&payload, "").unwrap();
        let decrypted = decrypt_payload(&encrypted, "").unwrap();

        assert_eq!(decrypted, payload);
    }

    #[test]
    fn encrypted_config_file_writes_and_reads_json() {
        let path = std::env::temp_dir().join(format!(
            "printbridge-config-transfer-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let payload = build_transfer_payload(&sample_config(), &ExportConfigOptions::all());
        let encrypted = encrypt_payload(&payload, "").unwrap();
        write_encrypted_file(&path, &encrypted).unwrap();
        let read = read_encrypted_file(&path).unwrap();

        assert_eq!(read, encrypted);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn encrypted_config_file_hash_changes_with_content() {
        let path = std::env::temp_dir().join(format!(
            "printbridge-config-transfer-hash-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let payload = build_transfer_payload(&sample_config(), &ExportConfigOptions::all());
        let first = encrypt_payload(&payload, "first").unwrap();
        write_encrypted_file(&path, &first).unwrap();
        let (_, first_hash) = read_encrypted_file_with_hash(&path).unwrap();

        let second = encrypt_payload(&payload, "second").unwrap();
        write_encrypted_file(&path, &second).unwrap();
        let (_, second_hash) = read_encrypted_file_with_hash(&path).unwrap();

        assert_ne!(first_hash, second_hash);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decrypt_rejects_wrong_password() {
        let payload = build_transfer_payload(&sample_config(), &ExportConfigOptions::all());
        let encrypted = encrypt_payload(&payload, "right").unwrap();

        let error = decrypt_payload(&encrypted, "wrong").unwrap_err();

        assert!(matches!(
            error,
            ConfigTransferError::InvalidPasswordOrPayload
        ));
    }

    #[test]
    fn decrypts_php_compatible_test_vector() {
        let encrypted = EncryptedConfigFile {
            format: "printbridge-config-encrypted".to_string(),
            version: 1,
            crypto: CryptoMetadata {
                kdf: "argon2id13".to_string(),
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
                cipher: "aes-256-gcm".to_string(),
                tag_bytes: 16,
                salt: "AAECAwQFBgcICQoLDA0ODw==".to_string(),
                nonce: "EBESExQVFhcYGRob".to_string(),
            },
            payload: "OTX3vkYug76bv335qmWdp85pbgu85QfwarlnqhxGoV0U+4sRez0dlwWy+5eIe597KLRqdHg7XJVbjLds/mXROcLHLhTJrJJ+DWpB2Xc6BX2sKii+bziOsb8akhUwxqo=".to_string(),
        };

        let payload = decrypt_payload(&encrypted, "test-password").unwrap();

        assert_eq!(payload.format, "printbridge-config");
        assert_eq!(payload.version, 1);
        assert_eq!(payload.config.service.unwrap().port, Some(17890));
    }

    #[test]
    fn merge_payload_preserves_token_when_missing_null_or_empty() {
        let mut current = sample_config();
        current.remote.bearer_token = Some("existing-token".to_string());

        let mut payload = build_transfer_payload(&current, &ExportConfigOptions::all());
        payload.config.remote.as_mut().unwrap().bearer_token = None;
        let merged = merge_payload(&current, &payload).unwrap();
        assert_eq!(
            merged.remote.bearer_token.as_deref(),
            Some("existing-token")
        );

        payload.config.remote.as_mut().unwrap().bearer_token = Some(None);
        let merged = merge_payload(&current, &payload).unwrap();
        assert_eq!(
            merged.remote.bearer_token.as_deref(),
            Some("existing-token")
        );

        payload.config.remote.as_mut().unwrap().bearer_token = Some(Some(String::new()));
        let merged = merge_payload(&current, &payload).unwrap();
        assert_eq!(
            merged.remote.bearer_token.as_deref(),
            Some("existing-token")
        );
    }

    #[test]
    fn merge_payload_overwrites_token_when_non_empty() {
        let mut current = sample_config();
        current.remote.bearer_token = Some("existing-token".to_string());
        let mut payload = build_transfer_payload(&current, &ExportConfigOptions::all());
        payload.config.remote.as_mut().unwrap().bearer_token = Some(Some("new-token".to_string()));

        let merged = merge_payload(&current, &payload).unwrap();

        assert_eq!(merged.remote.bearer_token.as_deref(), Some("new-token"));
    }

    #[test]
    fn merge_payload_clears_endpoint_url_when_imported_null() {
        let mut current = sample_config();
        current.remote.endpoint_url = Some("https://old.example.com/tasks".to_string());
        let payload = serde_json::from_str::<ConfigTransferPayload>(
            r#"{"format":"printbridge-config","version":1,"config":{"remote":{"endpoint_url":null}}}"#,
        )
        .unwrap();

        assert_eq!(
            payload.config.remote.as_ref().unwrap().endpoint_url,
            Some(None)
        );

        let merged = merge_payload(&current, &payload).unwrap();

        assert_eq!(merged.remote.endpoint_url, None);
    }

    #[test]
    fn merge_payload_replaces_allowed_origins_as_list() {
        let mut current = sample_config();
        current.security.allowed_origins = vec!["https://old.example.com".to_string()];
        let mut payload = build_transfer_payload(&current, &ExportConfigOptions::all());
        payload.config.security.as_mut().unwrap().allowed_origins =
            Some(vec!["https://new.example.com".to_string()]);

        let merged = merge_payload(&current, &payload).unwrap();

        assert_eq!(
            merged.security.allowed_origins,
            vec!["https://new.example.com".to_string()]
        );
    }

    #[test]
    fn merge_payload_rejects_invalid_values() {
        let current = sample_config();

        let mut invalid_origin = build_transfer_payload(&current, &ExportConfigOptions::all());
        invalid_origin
            .config
            .security
            .as_mut()
            .unwrap()
            .allowed_origins = Some(vec!["not-a-url".to_string()]);
        assert!(matches!(
            merge_payload(&current, &invalid_origin).unwrap_err(),
            ConfigTransferError::InvalidField(_)
        ));

        let mut invalid_poll = build_transfer_payload(&current, &ExportConfigOptions::all());
        invalid_poll
            .config
            .remote
            .as_mut()
            .unwrap()
            .poll_interval_seconds = Some(0);
        assert!(matches!(
            merge_payload(&current, &invalid_poll).unwrap_err(),
            ConfigTransferError::InvalidField(_)
        ));
    }

    #[test]
    fn preview_payload_hides_token_value() {
        let current = sample_config();
        let payload = build_transfer_payload(&current, &ExportConfigOptions::all());

        let preview = preview_payload(&current, &payload).unwrap();

        let token = preview
            .items
            .iter()
            .find(|item| item.key == "remote.bearer_token")
            .unwrap();
        assert_eq!(token.current, "已设置");
        assert_eq!(token.next, "将覆盖");
    }
}
