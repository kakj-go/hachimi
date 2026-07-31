use std::{
    collections::BTreeMap,
    io::Write as _,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt as _;
use hachimi_protocol::EnterprisePlatform;
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use sha2::Digest as _;
use thiserror::Error;
use zeroize::Zeroize;

use crate::{EnterpriseCredential, EnterpriseStreamEndpoint};

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnterpriseDownloadReceipt {
    pub content_type: Option<String>,
    pub content_hash: String,
    pub byte_size: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EnterpriseApiError {
    #[error("enterprise credential is invalid")]
    InvalidCredential,
    #[error("enterprise API request is invalid")]
    InvalidRequest,
    #[error("enterprise API authentication failed")]
    Authentication,
    #[error("enterprise API rate limit is active")]
    RateLimited { retry_after_ms: Option<i64> },
    #[error("enterprise API rejected the operation: {code}")]
    Provider { code: String, retryable: bool },
    #[error("enterprise API transport failed")]
    Transport,
    #[error("enterprise mutation outcome is indeterminate")]
    Indeterminate,
    #[error("enterprise API response is malformed")]
    MalformedResponse,
}

impl EnterpriseApiError {
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. }
                | Self::Transport
                | Self::Provider {
                    retryable: true,
                    ..
                }
        )
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidCredential => "enterprise_invalid_credential",
            Self::InvalidRequest => "enterprise_invalid_request",
            Self::Authentication => "enterprise_authentication_failed",
            Self::RateLimited { .. } => "enterprise_rate_limited",
            Self::Provider { .. } => "enterprise_provider_rejected",
            Self::Transport => "enterprise_transport_failed",
            Self::Indeterminate => "enterprise_outcome_indeterminate",
            Self::MalformedResponse => "enterprise_response_malformed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnterpriseDirectoryPage {
    pub items: Vec<Value>,
    pub next_page_token: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnterpriseMessageTarget {
    pub peer: String,
    pub thread: Option<String>,
    pub group: bool,
}

#[derive(Clone)]
pub struct EnterpriseApiClient {
    client: Client,
    endpoints: EnterpriseEndpoints,
    tokens: Arc<Mutex<BTreeMap<String, AccessToken>>>,
}

impl std::fmt::Debug for EnterpriseApiClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnterpriseApiClient")
            .field(
                "cached_accounts",
                &self.tokens.lock().map(|value| value.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct EnterpriseEndpoints {
    wecom: String,
    dingtalk_legacy: String,
    dingtalk_openapi: String,
    feishu: String,
}

struct AccessToken {
    value: String,
    expires_at_ms: i64,
}

impl Drop for AccessToken {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

impl Default for EnterpriseApiClient {
    fn default() -> Self {
        Self::new().expect("fixed enterprise HTTP client configuration")
    }
}

impl EnterpriseApiClient {
    pub fn new() -> Result<Self, EnterpriseApiError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("hachimi-agent/0.3 enterprise-connectors")
            .build()
            .map_err(|_| EnterpriseApiError::Transport)?;
        Ok(Self {
            client,
            endpoints: EnterpriseEndpoints {
                wecom: "https://qyapi.weixin.qq.com".into(),
                dingtalk_legacy: "https://oapi.dingtalk.com".into(),
                dingtalk_openapi: "https://api.dingtalk.com".into(),
                feishu: "https://open.feishu.cn".into(),
            },
            tokens: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Builds the production transport against a loopback-only endpoint.
    ///
    /// This is intentionally narrower than a generic endpoint override: local
    /// integration tests can exercise the real HTTP, authentication and
    /// streaming paths without making arbitrary clear-text remote hosts
    /// configurable in the product.
    pub fn with_loopback_endpoint(endpoint: &str) -> Result<Self, EnterpriseApiError> {
        let parsed =
            reqwest::Url::parse(endpoint).map_err(|_| EnterpriseApiError::InvalidRequest)?;
        let loopback = parsed
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|host| host.is_loopback());
        if !matches!(parsed.scheme(), "http" | "https")
            || !loopback
            || parsed.username() != ""
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(EnterpriseApiError::InvalidRequest);
        }
        let endpoint = endpoint.trim_end_matches('/').to_owned();
        let mut client = Self::new()?;
        client.endpoints = EnterpriseEndpoints {
            wecom: endpoint.clone(),
            dingtalk_legacy: endpoint.clone(),
            dingtalk_openapi: endpoint.clone(),
            feishu: endpoint,
        };
        Ok(client)
    }

    #[cfg(test)]
    fn with_single_endpoint(endpoint: &str) -> Self {
        Self::with_loopback_endpoint(endpoint).expect("loopback client")
    }

    pub async fn account_identity(
        &self,
        account_id: &str,
        credential: &EnterpriseCredential,
    ) -> Result<Value, EnterpriseApiError> {
        validate_account_id(account_id)?;
        let _ = self.access_token(account_id, credential).await?;
        Ok(json!({
            "platform": credential.platform(),
            "tenantId": credential.tenant_id(),
            "ingressMode": credential.ingress_mode(),
        }))
    }

    pub async fn download_attachment_to(
        &self,
        account_id: &str,
        credential: &EnterpriseCredential,
        event_id: &str,
        remote_id: &str,
        destination: &Path,
        max_bytes: u64,
    ) -> Result<EnterpriseDownloadReceipt, EnterpriseApiError> {
        validate_account_id(account_id)?;
        validate_remote_identity(event_id)?;
        validate_remote_identity(remote_id)?;
        if max_bytes == 0 {
            return Err(EnterpriseApiError::InvalidRequest);
        }
        let token = self.access_token(account_id, credential).await?;
        let response = match credential.platform() {
            EnterprisePlatform::Wecom => self
                .client
                .get(format!("{}/cgi-bin/media/get", self.endpoints.wecom))
                .query(&[("access_token", token.as_str()), ("media_id", remote_id)])
                .send()
                .await
                .map_err(|_| EnterpriseApiError::Transport)?,
            EnterprisePlatform::DingTalk => {
                let robot_code = credential
                    .robot_code()
                    .ok_or(EnterpriseApiError::InvalidCredential)?;
                let response = self
                    .client
                    .post(format!(
                        "{}/v1.0/robot/messageFiles/download",
                        self.endpoints.dingtalk_openapi
                    ))
                    .header("x-acs-dingtalk-access-token", &token)
                    .json(&json!({"downloadCode": remote_id, "robotCode": robot_code}))
                    .send()
                    .await
                    .map_err(|_| EnterpriseApiError::Transport)?;
                let value = parse_http_response(response).await?;
                let url = value
                    .get("downloadUrl")
                    .or_else(|| value.pointer("/result/downloadUrl"))
                    .and_then(Value::as_str)
                    .ok_or(EnterpriseApiError::MalformedResponse)?;
                let parsed =
                    reqwest::Url::parse(url).map_err(|_| EnterpriseApiError::MalformedResponse)?;
                if parsed.scheme() != "https" && parsed.host_str() != Some("127.0.0.1") {
                    return Err(EnterpriseApiError::MalformedResponse);
                }
                self.client
                    .get(parsed)
                    .send()
                    .await
                    .map_err(|_| EnterpriseApiError::Transport)?
            }
            EnterprisePlatform::Feishu => {
                let mut url = reqwest::Url::parse(&format!(
                    "{}/open-apis/im/v1/messages",
                    self.endpoints.feishu
                ))
                .map_err(|_| EnterpriseApiError::InvalidRequest)?;
                url.path_segments_mut()
                    .map_err(|_| EnterpriseApiError::InvalidRequest)?
                    .extend([event_id, "resources", remote_id]);
                self.client
                    .get(url)
                    .query(&[("type", "file")])
                    .bearer_auth(&token)
                    .send()
                    .await
                    .map_err(|_| EnterpriseApiError::Transport)?
            }
        };
        stream_response_to_file(response, destination, max_bytes).await
    }

    pub async fn stream_endpoint(
        &self,
        credential: &EnterpriseCredential,
    ) -> Result<EnterpriseStreamEndpoint, EnterpriseApiError> {
        let (client_id, client_secret) = credential.auth_pair();
        let (url, body) = match credential.platform() {
            EnterprisePlatform::DingTalk => (
                format!(
                    "{}/v1.0/gateway/connections/open",
                    self.endpoints.dingtalk_openapi
                ),
                json!({
                    "clientId": client_id,
                    "clientSecret": client_secret,
                    "subscriptions": [
                        {"type": "CALLBACK", "topic": "*"},
                        {"type": "SYSTEM", "topic": "disconnect"},
                        {"type": "SYSTEM", "topic": "ping"}
                    ]
                }),
            ),
            EnterprisePlatform::Feishu => (
                format!("{}/callback/ws/endpoint", self.endpoints.feishu),
                json!({"AppID": client_id, "AppSecret": client_secret}),
            ),
            EnterprisePlatform::Wecom => return Err(EnterpriseApiError::InvalidRequest),
        };
        let value = parse_http_response(
            self.client
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|_| EnterpriseApiError::Transport)?,
        )
        .await?;
        EnterpriseStreamEndpoint::from_bootstrap(credential.platform(), &value)
            .map_err(|_| EnterpriseApiError::MalformedResponse)
    }

    pub async fn departments(
        &self,
        account_id: &str,
        credential: &EnterpriseCredential,
        parent_id: Option<&str>,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<EnterpriseDirectoryPage, EnterpriseApiError> {
        validate_account_id(account_id)?;
        let page_size = page_size
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let response = self
            .execute_authenticated(account_id, credential, false, |token| {
                match credential.platform() {
                    EnterprisePlatform::Wecom => {
                        let mut request = self
                            .client
                            .get(format!("{}/cgi-bin/department/list", self.endpoints.wecom));
                        request = request.query(&[("access_token", token)]);
                        if let Some(parent_id) = parent_id {
                            request = request.query(&[("id", parent_id)]);
                        }
                        request
                    }
                    EnterprisePlatform::DingTalk => self
                        .client
                        .post(format!(
                            "{}/topapi/v2/department/listsub",
                            self.endpoints.dingtalk_legacy
                        ))
                        .query(&[("access_token", token)])
                        .json(&json!({"dept_id": parent_id.unwrap_or("1")})),
                    EnterprisePlatform::Feishu => {
                        let parent_id = parent_id.unwrap_or("0");
                        let mut request = self
                            .client
                            .get(format!(
                                "{}/open-apis/contact/v3/departments/{parent_id}/children",
                                self.endpoints.feishu
                            ))
                            .bearer_auth(token)
                            .query(&[("page_size", page_size.to_string())]);
                        if let Some(page_token) = page_token {
                            request = request.query(&[("page_token", page_token)]);
                        }
                        request
                    }
                }
            })
            .await?;
        parse_directory_page(credential.platform(), response, "department")
    }

    pub async fn members(
        &self,
        account_id: &str,
        credential: &EnterpriseCredential,
        department_id: &str,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<EnterpriseDirectoryPage, EnterpriseApiError> {
        validate_account_id(account_id)?;
        if department_id.trim().is_empty() || department_id.len() > 256 {
            return Err(EnterpriseApiError::InvalidRequest);
        }
        let page_size = page_size
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let response = self
            .execute_authenticated(account_id, credential, false, |token| {
                match credential.platform() {
                    EnterprisePlatform::Wecom => self
                        .client
                        .get(format!("{}/cgi-bin/user/list", self.endpoints.wecom))
                        .query(&[
                            ("access_token", token),
                            ("department_id", department_id),
                            ("fetch_child", "0"),
                        ]),
                    EnterprisePlatform::DingTalk => self
                        .client
                        .post(format!(
                            "{}/topapi/v2/user/list",
                            self.endpoints.dingtalk_legacy
                        ))
                        .query(&[("access_token", token)])
                        .json(&json!({
                            "dept_id": department_id,
                            "cursor": page_token.and_then(|value| value.parse::<u64>().ok()).unwrap_or(0),
                            "size": page_size,
                        })),
                    EnterprisePlatform::Feishu => {
                        let mut request = self
                            .client
                            .get(format!(
                                "{}/open-apis/contact/v3/users/find_by_department",
                                self.endpoints.feishu
                            ))
                            .bearer_auth(token)
                            .query(&[
                                ("department_id", department_id.to_owned()),
                                ("page_size", page_size.to_string()),
                            ]);
                        if let Some(page_token) = page_token {
                            request = request.query(&[("page_token", page_token)]);
                        }
                        request
                    }
                }
            })
            .await?;
        parse_directory_page(credential.platform(), response, "member")
    }

    pub async fn send_text(
        &self,
        account_id: &str,
        credential: &EnterpriseCredential,
        target: &EnterpriseMessageTarget,
        text: &str,
        idempotency_key: &str,
    ) -> Result<Value, EnterpriseApiError> {
        validate_account_id(account_id)?;
        validate_message(target, text, idempotency_key)?;
        let feishu_content = serde_json::to_string(&json!({"text": text}))
            .map_err(|_| EnterpriseApiError::InvalidRequest)?;
        let dingtalk_content = serde_json::to_string(&json!({"content": text}))
            .map_err(|_| EnterpriseApiError::InvalidRequest)?;
        let agent_id = if credential.platform() == EnterprisePlatform::Wecom {
            Some(
                credential
                    .agent_id()
                    .ok_or(EnterpriseApiError::InvalidCredential)?,
            )
        } else {
            None
        };
        let robot_code = if credential.platform() == EnterprisePlatform::DingTalk {
            Some(
                credential
                    .robot_code()
                    .ok_or(EnterpriseApiError::InvalidCredential)?
                    .to_owned(),
            )
        } else {
            None
        };
        let response = self
            .execute_authenticated(account_id, credential, true, |token| match credential.platform() {
            EnterprisePlatform::Wecom => {
                let (path, body) = if target.group {
                    (
                        "/cgi-bin/appchat/send",
                        json!({
                            "chatid": target.thread.as_deref().unwrap_or(&target.peer),
                            "msgtype": "text",
                            "text": {"content": text},
                            "safe": 0,
                        }),
                    )
                } else {
                    (
                        "/cgi-bin/message/send",
                        json!({
                            "touser": target.peer,
                            "msgtype": "text",
                            "agentid": agent_id.unwrap_or_default(),
                            "text": {"content": text},
                            "safe": 0,
                            "enable_id_trans": 0,
                        }),
                    )
                };
                self.client
                    .post(format!("{}{path}", self.endpoints.wecom))
                    .query(&[("access_token", token)])
                    .header("X-Hachimi-Idempotency-Key", idempotency_key)
                    .json(&body)
            }
            EnterprisePlatform::DingTalk => {
                let (path, body) = if target.group {
                    (
                        "/v1.0/robot/groupMessages/send",
                        json!({
                            "robotCode": robot_code.as_deref().unwrap_or_default(),
                            "openConversationId": target.thread.as_deref().unwrap_or(&target.peer),
                            "msgKey": "sampleText",
                            "msgParam": dingtalk_content.clone(),
                        }),
                    )
                } else {
                    (
                        "/v1.0/robot/oToMessages/batchSend",
                        json!({
                            "robotCode": robot_code.as_deref().unwrap_or_default(),
                            "userIds": [target.peer.as_str()],
                            "msgKey": "sampleText",
                            "msgParam": dingtalk_content.clone(),
                        }),
                    )
                };
                self.client
                    .post(format!("{}{path}", self.endpoints.dingtalk_openapi))
                    .header("x-acs-dingtalk-access-token", token)
                    .header("X-Hachimi-Idempotency-Key", idempotency_key)
                    .json(&body)
            }
            EnterprisePlatform::Feishu => {
                let receive_id_type = if target.group { "chat_id" } else { "open_id" };
                let receive_id = target
                    .thread
                    .as_deref()
                    .filter(|_| target.group)
                    .unwrap_or(&target.peer);
                self.client
                    .post(format!(
                        "{}/open-apis/im/v1/messages",
                        self.endpoints.feishu
                    ))
                    .bearer_auth(token)
                    .query(&[("receive_id_type", receive_id_type)])
                    .header("X-Hachimi-Idempotency-Key", idempotency_key)
                    .json(&json!({
                        "receive_id": receive_id,
                        "msg_type": "text",
                        "content": feishu_content.clone(),
                        "uuid": idempotency_key,
                    }))
            }
        })
            .await?;
        validate_provider_response(credential.platform(), response)
    }

    pub fn revoke(&self, account_id: &str) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(account_id);
        }
    }

    async fn access_token(
        &self,
        account_id: &str,
        credential: &EnterpriseCredential,
    ) -> Result<String, EnterpriseApiError> {
        let now = now_ms();
        if let Some(value) = self
            .tokens
            .lock()
            .map_err(|_| EnterpriseApiError::Transport)?
            .get(account_id)
            .filter(|token| token.expires_at_ms.saturating_sub(60_000) > now)
            .map(|token| token.value.clone())
        {
            return Ok(value);
        }
        let (identity, secret) = credential.auth_pair();
        if identity.trim().is_empty() || secret.trim().is_empty() {
            return Err(EnterpriseApiError::InvalidCredential);
        }
        let response = match credential.platform() {
            EnterprisePlatform::Wecom => {
                self.execute_json(
                    self.client
                        .get(format!("{}/cgi-bin/gettoken", self.endpoints.wecom))
                        .query(&[("corpid", identity), ("corpsecret", secret)]),
                    false,
                )
                .await?
            }
            EnterprisePlatform::DingTalk => {
                self.execute_json(
                    self.client
                        .get(format!("{}/gettoken", self.endpoints.dingtalk_legacy))
                        .query(&[("appkey", identity), ("appsecret", secret)]),
                    false,
                )
                .await?
            }
            EnterprisePlatform::Feishu => {
                self.execute_json(
                    self.client
                        .post(format!(
                            "{}/open-apis/auth/v3/tenant_access_token/internal",
                            self.endpoints.feishu
                        ))
                        .json(&json!({"app_id": identity, "app_secret": secret})),
                    false,
                )
                .await?
            }
        };
        let response = validate_provider_response(credential.platform(), response)?;
        let token = response
            .get("access_token")
            .or_else(|| response.get("tenant_access_token"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 16 * 1024)
            .ok_or(EnterpriseApiError::MalformedResponse)?;
        let expires_in = response
            .get("expires_in")
            .or_else(|| response.get("expire"))
            .and_then(Value::as_i64)
            .unwrap_or(7_200)
            .clamp(60, 86_400);
        let value = token.to_owned();
        self.tokens
            .lock()
            .map_err(|_| EnterpriseApiError::Transport)?
            .insert(
                account_id.to_owned(),
                AccessToken {
                    value: value.clone(),
                    expires_at_ms: now.saturating_add(expires_in.saturating_mul(1_000)),
                },
            );
        Ok(value)
    }

    async fn execute_json(
        &self,
        request: reqwest::RequestBuilder,
        mutation: bool,
    ) -> Result<Value, EnterpriseApiError> {
        let response = request.send().await.map_err(|error| {
            if mutation && (error.is_timeout() || error.is_connect()) {
                EnterpriseApiError::Indeterminate
            } else {
                EnterpriseApiError::Transport
            }
        })?;
        parse_http_response(response).await
    }

    async fn execute_authenticated<F>(
        &self,
        account_id: &str,
        credential: &EnterpriseCredential,
        mutation: bool,
        build: F,
    ) -> Result<Value, EnterpriseApiError>
    where
        F: Fn(&str) -> reqwest::RequestBuilder,
    {
        for refresh_attempt in 0..=1 {
            let token = self.access_token(account_id, credential).await?;
            let result = self
                .execute_json(build(&token), mutation)
                .await
                .and_then(|value| validate_provider_response(credential.platform(), value));
            match result {
                Err(EnterpriseApiError::Authentication) if refresh_attempt == 0 => {
                    self.revoke(account_id);
                }
                other => return other,
            }
        }
        Err(EnterpriseApiError::Authentication)
    }
}

fn validate_remote_identity(value: &str) -> Result<(), EnterpriseApiError> {
    if value.is_empty()
        || value.len() > 1024
        || value.contains(['\0', '\r', '\n'])
        || value == "."
        || value == ".."
    {
        Err(EnterpriseApiError::InvalidRequest)
    } else {
        Ok(())
    }
}

async fn stream_response_to_file(
    response: Response,
    destination: &Path,
    max_bytes: u64,
) -> Result<EnterpriseDownloadReceipt, EnterpriseApiError> {
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(EnterpriseApiError::Authentication);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(EnterpriseApiError::RateLimited {
            retry_after_ms: response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<i64>().ok())
                .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1_000))),
        });
    }
    if !status.is_success() {
        return Err(EnterpriseApiError::Provider {
            code: format!("http_{}", status.as_u16()),
            retryable: status.is_server_error(),
        });
    }
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes)
    {
        return Err(EnterpriseApiError::InvalidRequest);
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase()
        });
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|_| EnterpriseApiError::Transport)?;
    let mut stream = response.bytes_stream();
    let mut size = 0_u64;
    let mut hasher = sha2::Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| EnterpriseApiError::Transport)?;
        size = size
            .checked_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX))
            .ok_or(EnterpriseApiError::InvalidRequest)?;
        if size > max_bytes {
            return Err(EnterpriseApiError::InvalidRequest);
        }
        sha2::Digest::update(&mut hasher, &chunk);
        file.write_all(&chunk)
            .map_err(|_| EnterpriseApiError::Transport)?;
    }
    file.sync_all().map_err(|_| EnterpriseApiError::Transport)?;
    Ok(EnterpriseDownloadReceipt {
        content_type,
        content_hash: sha2::Digest::finalize(hasher)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        byte_size: size,
    })
}

fn validate_account_id(account_id: &str) -> Result<(), EnterpriseApiError> {
    if account_id.trim().is_empty() || account_id.len() > 128 {
        Err(EnterpriseApiError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn validate_message(
    target: &EnterpriseMessageTarget,
    text: &str,
    idempotency_key: &str,
) -> Result<(), EnterpriseApiError> {
    if target.peer.trim().is_empty()
        || target.peer.len() > 512
        || target
            .thread
            .as_deref()
            .is_some_and(|value| value.len() > 512)
        || text.trim().is_empty()
        || text.chars().count() > 20_000
        || idempotency_key.trim().is_empty()
        || idempotency_key.len() > 128
    {
        return Err(EnterpriseApiError::InvalidRequest);
    }
    Ok(())
}

async fn parse_http_response(response: Response) -> Result<Value, EnterpriseApiError> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after_ms = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .map(|seconds| seconds.saturating_mul(1_000));
        return Err(EnterpriseApiError::RateLimited { retry_after_ms });
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(EnterpriseApiError::Authentication);
    }
    if !status.is_success() {
        return Err(EnterpriseApiError::Provider {
            code: format!("http_{}", status.as_u16()),
            retryable: status.is_server_error(),
        });
    }
    response
        .json::<Value>()
        .await
        .map_err(|_| EnterpriseApiError::MalformedResponse)
}

fn validate_provider_response(
    platform: EnterprisePlatform,
    value: Value,
) -> Result<Value, EnterpriseApiError> {
    let (code, message) = match platform {
        EnterprisePlatform::Wecom | EnterprisePlatform::DingTalk => (
            value.get("errcode").and_then(Value::as_i64).unwrap_or(0),
            value.get("errmsg").and_then(Value::as_str),
        ),
        EnterprisePlatform::Feishu => (
            value.get("code").and_then(Value::as_i64).unwrap_or(0),
            value.get("msg").and_then(Value::as_str),
        ),
    };
    if code == 0 {
        return Ok(value);
    }
    let authentication = matches!(code, 40014 | 42001 | 88 | 99991663 | 99991664);
    if authentication {
        return Err(EnterpriseApiError::Authentication);
    }
    let retryable = matches!(code, -1 | 45009 | 130101 | 99991400)
        || message.is_some_and(|message| message.to_ascii_lowercase().contains("rate"));
    Err(EnterpriseApiError::Provider {
        code: format!("provider_{code}"),
        retryable,
    })
}

fn parse_directory_page(
    platform: EnterprisePlatform,
    response: Value,
    kind: &str,
) -> Result<EnterpriseDirectoryPage, EnterpriseApiError> {
    let response = validate_provider_response(platform, response)?;
    let root = response.get("result").or_else(|| response.get("data"));
    let candidates = match (platform, kind) {
        (EnterprisePlatform::Wecom, "department") => response.get("department"),
        (EnterprisePlatform::Wecom, _) => response.get("userlist"),
        (EnterprisePlatform::DingTalk, "department") => root.and_then(|root| root.get("result")),
        (EnterprisePlatform::DingTalk, _) => root.and_then(|root| root.get("list")),
        (EnterprisePlatform::Feishu, _) => root.and_then(|root| root.get("items")),
    };
    let items = candidates
        .and_then(Value::as_array)
        .cloned()
        .ok_or(EnterpriseApiError::MalformedResponse)?;
    if items.len() > 1_000 {
        return Err(EnterpriseApiError::MalformedResponse);
    }
    let next_page_token = root
        .and_then(|root| root.get("page_token"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            root.and_then(|root| root.get("next_cursor"))
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
        });
    let has_more = root
        .and_then(|root| root.get("has_more"))
        .and_then(Value::as_bool)
        .unwrap_or(next_page_token.is_some());
    Ok(EnterpriseDirectoryPage {
        items,
        next_page_token,
        has_more,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read as _, net::TcpListener, thread};

    fn chunked_fixture(chunks: Vec<Vec<u8>>, complete: bool) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).expect("request");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                .expect("headers");
            for chunk in chunks {
                stream
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .expect("chunk size");
                stream.write_all(&chunk).expect("chunk");
                stream.write_all(b"\r\n").expect("chunk end");
                stream.flush().expect("flush");
            }
            if complete {
                stream.write_all(b"0\r\n\r\n").expect("complete");
                stream.flush().expect("complete flush");
            }
        });
        (format!("http://{address}/download"), server)
    }

    #[test]
    fn messages_are_bounded_before_network_dispatch() {
        let target = EnterpriseMessageTarget {
            peer: "peer".into(),
            thread: None,
            group: false,
        };
        assert!(validate_message(&target, "hello", "key").is_ok());
        assert!(validate_message(&target, "", "key").is_err());
        assert!(validate_message(&target, "hello", "").is_err());
    }

    #[test]
    fn endpoint_override_never_changes_the_platform_contract() {
        let client = EnterpriseApiClient::with_single_endpoint("http://127.0.0.1:1");
        assert_eq!(client.endpoints.wecom, "http://127.0.0.1:1");
        assert_eq!(client.endpoints.feishu, "http://127.0.0.1:1");
        assert!(matches!(
            EnterpriseApiClient::with_loopback_endpoint("http://example.com"),
            Err(EnterpriseApiError::InvalidRequest)
        ));
        assert!(matches!(
            EnterpriseApiClient::with_loopback_endpoint("file:///tmp/fixture"),
            Err(EnterpriseApiError::InvalidRequest)
        ));
    }

    #[tokio::test]
    async fn attachment_fixture_streams_chunks_and_hashes_the_complete_file() {
        let chunks = vec![b"%PDF-1.7\n".to_vec(), b"streamed-fixture".to_vec()];
        let expected = chunks.concat();
        let (url, server) = chunked_fixture(chunks, true);
        let response = Client::new().get(url).send().await.expect("response");
        let root = tempfile::tempdir().expect("root");
        let destination = root.path().join("download.part");
        let receipt = stream_response_to_file(response, &destination, 1024)
            .await
            .expect("download");
        server.join().expect("server");
        assert_eq!(std::fs::read(&destination).expect("file"), expected);
        assert_eq!(receipt.byte_size, expected.len() as u64);
        assert_eq!(receipt.content_type.as_deref(), Some("application/pdf"));
        let expected_hash = sha2::Sha256::digest(&expected)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(receipt.content_hash, expected_hash);
    }

    #[tokio::test]
    async fn attachment_fixture_reports_midstream_disconnect_as_transport_failure() {
        let (url, server) = chunked_fixture(vec![b"%PDF-partial".to_vec()], false);
        let response = Client::new().get(url).send().await.expect("response");
        let root = tempfile::tempdir().expect("root");
        let destination = root.path().join("download.part");
        assert_eq!(
            stream_response_to_file(response, &destination, 1024).await,
            Err(EnterpriseApiError::Transport)
        );
        server.join().expect("server");
        assert!(destination.is_file());
        assert!(std::fs::metadata(destination).expect("partial").len() > 0);
    }
}
