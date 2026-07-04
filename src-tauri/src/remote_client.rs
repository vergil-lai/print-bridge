use crate::{
    config::RemoteConfig,
    remote_protocol::{parse_remote_tasks, RemoteProtocolError, RemoteTask},
    remote_store::RemoteStatusEvent,
};
use reqwest::{RequestBuilder, Url};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RemoteClient {
    client: reqwest::Client,
}

#[derive(Debug, Error)]
pub enum RemoteClientError {
    #[error("remote endpoint url is missing")]
    MissingEndpoint,
    #[error("invalid remote endpoint url")]
    InvalidEndpoint(#[from] url::ParseError),
    #[error("remote server returned HTTP {0}")]
    HttpStatus(u16),
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Protocol(#[from] RemoteProtocolError),
}

impl Default for RemoteClient {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl RemoteClient {
    pub async fn fetch_tasks(
        &self,
        config: &RemoteConfig,
    ) -> Result<Vec<RemoteTask>, RemoteClientError> {
        let url = endpoint_url(config)?;
        let response = with_common_headers(self.client.get(url), config, false)
            .send()
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Vec::new());
        }
        if !status.is_success() {
            return Err(RemoteClientError::HttpStatus(status.as_u16()));
        }

        parse_remote_tasks(&response.text().await?).map_err(RemoteClientError::Protocol)
    }

    pub async fn report_status(
        &self,
        config: &RemoteConfig,
        event: &RemoteStatusEvent,
    ) -> Result<(), RemoteClientError> {
        let url = endpoint_url(config)?;
        let response = with_common_headers(self.client.post(url), config, false)
            .json(&StatusReportBody::new(config, event))
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(RemoteClientError::HttpStatus(status.as_u16()))
        }
    }

    pub async fn test_connection(&self, config: &RemoteConfig) -> Result<(), RemoteClientError> {
        let url = endpoint_url(config)?;
        let get_response = with_common_headers(self.client.get(url.clone()), config, true)
            .send()
            .await?;
        if !get_response.status().is_success() {
            return Err(RemoteClientError::HttpStatus(
                get_response.status().as_u16(),
            ));
        }

        let post_response = with_common_headers(self.client.post(url), config, true)
            .json(&ConnectionTestBody::new(config))
            .send()
            .await?;
        if post_response.status().is_success() {
            Ok(())
        } else {
            Err(RemoteClientError::HttpStatus(
                post_response.status().as_u16(),
            ))
        }
    }
}

impl RemoteClientError {
    pub fn is_configuration_status(&self) -> bool {
        matches!(self, Self::HttpStatus(401 | 403 | 404))
    }
}

#[derive(Debug, Serialize)]
struct StatusReportBody<'a> {
    event: &'static str,
    event_id: &'a str,
    request_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_id: Option<&'a str>,
    job_id: &'a str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    occurred_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_name: Option<&'a str>,
}

impl<'a> StatusReportBody<'a> {
    fn new(config: &'a RemoteConfig, event: &'a RemoteStatusEvent) -> Self {
        Self {
            event: "status",
            event_id: &event.event_id,
            request_id: &event.request_id,
            batch_id: event.batch_id.as_deref(),
            job_id: &event.job_id,
            status: event.status.as_str(),
            message: event.message.as_deref(),
            occurred_at: &event.occurred_at,
            device_id: configured_value(config.device_id.as_deref()),
            device_name: configured_value(config.device_name.as_deref()),
        }
    }
}

#[derive(Debug, Serialize)]
struct ConnectionTestBody<'a> {
    event: &'static str,
    event_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_name: Option<&'a str>,
}

impl<'a> ConnectionTestBody<'a> {
    fn new(config: &'a RemoteConfig) -> Self {
        Self {
            event: "connection_test",
            event_id: Uuid::new_v4().to_string(),
            status: "test",
            device_id: configured_value(config.device_id.as_deref()),
            device_name: configured_value(config.device_name.as_deref()),
        }
    }
}

fn endpoint_url(config: &RemoteConfig) -> Result<Url, RemoteClientError> {
    let value = configured_value(config.endpoint_url.as_deref())
        .ok_or(RemoteClientError::MissingEndpoint)?;
    Ok(Url::parse(value)?)
}

fn with_common_headers(
    request: RequestBuilder,
    config: &RemoteConfig,
    is_test: bool,
) -> RequestBuilder {
    let mut request = request;

    if let Some(token) = configured_value(config.bearer_token.as_deref()) {
        request = request.bearer_auth(token);
    }
    if let Some(device_id) = configured_value(config.device_id.as_deref()) {
        request = request.header("X-PrintBridge-Device-Id", device_id);
    }
    if let Some(device_name) = configured_value(config.device_name.as_deref()) {
        request = request.header("X-PrintBridge-Device-Name", device_name);
    }
    if is_test {
        request = request.header("X-PrintBridge-Test", "true");
    }

    request
}

fn configured_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
