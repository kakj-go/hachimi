// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/rmcp-client/src/auth_status.rs,
// codex-rs/rmcp-client/src/perform_oauth_login.rs, codex-rs/rmcp-client/src/oauth.rs, and
// codex-rs/rmcp-client/src/oauth/refresh_transaction.rs.
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: bounded RFC discovery/exchange and Keyring-only credential handoff.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::oneshot;
use url::Url;

const MAX_METADATA_BYTES: usize = 256 * 1024;
const MAX_CALLBACK_BYTES: usize = 16 * 1024;
const DEFAULT_LOGIN_TIMEOUT_SECS: u64 = 300;
const REFRESH_SKEW_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, Error)]
pub enum McpOAuthError {
    #[error("MCP OAuth is not supported by this server")]
    Unsupported,
    #[error("MCP OAuth configuration is invalid")]
    InvalidConfiguration,
    #[error("MCP OAuth metadata discovery failed")]
    Discovery,
    #[error("MCP OAuth dynamic client registration failed")]
    Registration,
    #[error("MCP OAuth callback listener failed")]
    CallbackListener,
    #[error("MCP OAuth callback was invalid")]
    InvalidCallback,
    #[error("MCP OAuth callback state did not match")]
    StateMismatch,
    #[error("MCP OAuth login timed out")]
    TimedOut,
    #[error("MCP OAuth login was cancelled")]
    Cancelled,
    #[error("MCP OAuth provider rejected authorization")]
    ProviderRejected,
    #[error("MCP OAuth token exchange failed")]
    TokenExchange,
    #[error("MCP OAuth credential is missing or expired")]
    AuthorizationRequired,
    #[error("MCP OAuth credential refresh failed")]
    RefreshFailed,
    #[error("MCP OAuth credential serialization failed")]
    CredentialEncoding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthDiscovery {
    authorization_endpoint: Url,
    token_endpoint: Url,
    registration_endpoint: Option<Url>,
    scopes_supported: Vec<String>,
}

impl McpOAuthDiscovery {
    #[must_use]
    pub fn scopes_supported(&self) -> &[String] {
        &self.scopes_supported
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthCredential {
    server_url: String,
    token_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    scopes: Vec<String>,
    expires_at_ms: Option<u64>,
}

impl std::fmt::Debug for McpOAuthCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpOAuthCredential")
            .field("server_url", &self.server_url)
            .field("client_id_configured", &!self.client_id.is_empty())
            .field("refresh_token_configured", &self.refresh_token.is_some())
            .field("expires_at_ms", &self.expires_at_ms)
            .finish_non_exhaustive()
    }
}

impl McpOAuthCredential {
    pub fn to_secret_json(&self) -> Result<String, McpOAuthError> {
        serde_json::to_string(self).map_err(|_| McpOAuthError::CredentialEncoding)
    }

    pub fn from_secret_json(value: &str) -> Result<Self, McpOAuthError> {
        serde_json::from_str(value).map_err(|_| McpOAuthError::CredentialEncoding)
    }

    pub fn authorization_header(&self) -> Result<String, McpOAuthError> {
        if !self.token_type.eq_ignore_ascii_case("bearer") || self.access_token.trim().is_empty() {
            return Err(McpOAuthError::AuthorizationRequired);
        }
        Ok(format!("Bearer {}", self.access_token))
    }

    #[must_use]
    pub fn needs_refresh(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires_at| expires_at <= now_ms.saturating_add(REFRESH_SKEW_MS))
    }

    #[must_use]
    pub fn can_authorize(&self, now_ms: u64) -> bool {
        if !self.token_type.eq_ignore_ascii_case("bearer") || self.client_id.trim().is_empty() {
            return false;
        }
        if self.needs_refresh(now_ms) {
            return self
                .refresh_token
                .as_deref()
                .is_some_and(|token| !token.trim().is_empty());
        }
        !self.access_token.trim().is_empty()
    }

    #[must_use]
    pub fn server_url(&self) -> &str {
        &self.server_url
    }
}

pub struct McpOAuthLoginHandle {
    authorization_url: String,
    stop: Arc<AtomicBool>,
    completion: tokio::task::JoinHandle<Result<McpOAuthCredential, McpOAuthError>>,
}

impl std::fmt::Debug for McpOAuthLoginHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpOAuthLoginHandle")
            .field("authorization_url", &self.authorization_url)
            .finish_non_exhaustive()
    }
}

impl McpOAuthLoginHandle {
    #[must_use]
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    pub async fn wait(self) -> Result<McpOAuthCredential, McpOAuthError> {
        self.completion
            .await
            .map_err(|_| McpOAuthError::Cancelled)?
    }

    pub async fn cancel(self) -> Result<McpOAuthCredential, McpOAuthError> {
        self.stop.store(true, Ordering::Release);
        self.wait().await
    }
}

#[derive(Debug, Deserialize)]
struct ProtectedResourceMetadata {
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AuthorizationServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ClientRegistrationResponse {
    client_id: String,
    client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

#[derive(Debug)]
struct CallbackResult {
    code: String,
    state: String,
}

pub async fn discover_mcp_oauth(
    server_url: &str,
) -> Result<Option<McpOAuthDiscovery>, McpOAuthError> {
    let server = validate_endpoint(server_url)?;
    let client = oauth_http_client()?;
    let mut protected = None;
    for candidate in protected_resource_candidates(&server)? {
        if let Some(value) = get_optional_json(&client, candidate).await? {
            protected = Some(value);
            break;
        }
    }
    // Codex/RMCP also supports authorization-server metadata directly at the
    // MCP resource path. Protected Resource Metadata is preferred when
    // present, but its absence does not mean OAuth is unsupported.
    let authorization_server = protected
        .as_ref()
        .and_then(|metadata: &ProtectedResourceMetadata| metadata.authorization_servers.first())
        .map(String::as_str)
        .unwrap_or_else(|| server.as_str());
    let authorization_server = validate_endpoint(authorization_server)?;
    let mut metadata = None;
    for candidate in authorization_server_candidates(&authorization_server)? {
        if let Some(value) = get_optional_json(&client, candidate).await? {
            metadata = Some(value);
            break;
        }
    }
    let Some(metadata): Option<AuthorizationServerMetadata> = metadata else {
        return Ok(None);
    };
    if !metadata.code_challenge_methods_supported.is_empty()
        && !metadata
            .code_challenge_methods_supported
            .iter()
            .any(|method| method == "S256")
    {
        return Err(McpOAuthError::Unsupported);
    }
    let authorization_endpoint = validate_endpoint(&metadata.authorization_endpoint)?;
    let token_endpoint = validate_endpoint(&metadata.token_endpoint)?;
    let registration_endpoint = metadata
        .registration_endpoint
        .as_deref()
        .map(validate_endpoint)
        .transpose()?;
    Ok(Some(McpOAuthDiscovery {
        authorization_endpoint,
        token_endpoint,
        registration_endpoint,
        scopes_supported: normalize_scopes(
            metadata.scopes_supported.into_iter().chain(
                protected
                    .into_iter()
                    .flat_map(|metadata| metadata.scopes_supported),
            ),
        ),
    }))
}

pub async fn start_mcp_oauth_login(
    server_url: &str,
    requested_scopes: &[String],
    timeout_secs: Option<u32>,
) -> Result<McpOAuthLoginHandle, McpOAuthError> {
    let discovery = discover_mcp_oauth(server_url)
        .await?
        .ok_or(McpOAuthError::Unsupported)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| McpOAuthError::CallbackListener)?;
    listener
        .set_nonblocking(true)
        .map_err(|_| McpOAuthError::CallbackListener)?;
    let callback_id = URL_SAFE_NO_PAD.encode(&Sha256::digest(server_url.as_bytes())[..9]);
    let redirect_uri = format!(
        "http://127.0.0.1:{}/callback/{callback_id}",
        listener
            .local_addr()
            .map_err(|_| McpOAuthError::CallbackListener)?
            .port()
    );
    let registration_endpoint = discovery
        .registration_endpoint
        .as_ref()
        .ok_or(McpOAuthError::Unsupported)?;
    let client = oauth_http_client()?;
    let registration: ClientRegistrationResponse = post_json(
        &client,
        registration_endpoint.clone(),
        &serde_json::json!({
            "client_name": "Hachimi",
            "redirect_uris": [&redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }),
        McpOAuthError::Registration,
    )
    .await?;
    if registration.client_id.trim().is_empty() {
        return Err(McpOAuthError::Registration);
    }
    let verifier = random_urlsafe(3);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_urlsafe(2);
    let scopes = selected_scopes(requested_scopes, &discovery.scopes_supported)?;
    let mut authorization_url = discovery.authorization_endpoint.clone();
    authorization_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &registration.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("resource", server_url);
    if !scopes.is_empty() {
        authorization_url
            .query_pairs_mut()
            .append_pair("scope", &scopes.join(" "));
    }
    let expected_path = Url::parse(&redirect_uri)
        .map_err(|_| McpOAuthError::InvalidConfiguration)?
        .path()
        .to_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let (callback_sender, callback_receiver) = oneshot::channel();
    let callback_task =
        spawn_callback_listener(listener, expected_path, Arc::clone(&stop), callback_sender);
    let server_url = server_url.to_owned();
    let timeout = Duration::from_secs(
        u64::from(timeout_secs.unwrap_or(DEFAULT_LOGIN_TIMEOUT_SECS as u32)).clamp(1, 900),
    );
    let completion_stop = Arc::clone(&stop);
    let completion = tokio::spawn(async move {
        let result = complete_login(
            &client,
            callback_receiver,
            &state,
            &verifier,
            &redirect_uri,
            server_url,
            discovery,
            registration,
            scopes,
            timeout,
        )
        .await;
        completion_stop.store(true, Ordering::Release);
        let _ = callback_task.await;
        result
    });
    Ok(McpOAuthLoginHandle {
        authorization_url: authorization_url.to_string(),
        stop,
        completion,
    })
}

pub async fn refresh_mcp_oauth_credential(
    mut credential: McpOAuthCredential,
) -> Result<McpOAuthCredential, McpOAuthError> {
    if !credential.needs_refresh(epoch_ms()) {
        credential.authorization_header()?;
        return Ok(credential);
    }
    let refresh_token = credential
        .refresh_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or(McpOAuthError::AuthorizationRequired)?;
    let endpoint = validate_endpoint(&credential.token_endpoint)?;
    let mut form = BTreeMap::from([
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", refresh_token.to_owned()),
        ("client_id", credential.client_id.clone()),
    ]);
    if let Some(secret) = &credential.client_secret {
        form.insert("client_secret", secret.clone());
    }
    let response = post_form(
        &oauth_http_client()?,
        endpoint,
        &form,
        McpOAuthError::RefreshFailed,
    )
    .await?;
    install_token_response(&mut credential, response, true)?;
    Ok(credential)
}

#[allow(clippy::too_many_arguments)]
async fn complete_login(
    client: &reqwest::Client,
    callback: oneshot::Receiver<Result<CallbackResult, McpOAuthError>>,
    expected_state: &str,
    verifier: &str,
    redirect_uri: &str,
    server_url: String,
    discovery: McpOAuthDiscovery,
    registration: ClientRegistrationResponse,
    scopes: Vec<String>,
    timeout: Duration,
) -> Result<McpOAuthCredential, McpOAuthError> {
    let callback = tokio::time::timeout(timeout, callback)
        .await
        .map_err(|_| McpOAuthError::TimedOut)?
        .map_err(|_| McpOAuthError::Cancelled)??;
    if callback.state != expected_state {
        return Err(McpOAuthError::StateMismatch);
    }
    let mut form = BTreeMap::from([
        ("grant_type", "authorization_code".to_owned()),
        ("code", callback.code),
        ("client_id", registration.client_id.clone()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("code_verifier", verifier.to_owned()),
        ("resource", server_url.clone()),
    ]);
    if let Some(secret) = &registration.client_secret {
        form.insert("client_secret", secret.clone());
    }
    let response = post_form(
        client,
        discovery.token_endpoint.clone(),
        &form,
        McpOAuthError::TokenExchange,
    )
    .await?;
    let mut credential = McpOAuthCredential {
        server_url,
        token_endpoint: discovery.token_endpoint.to_string(),
        client_id: registration.client_id,
        client_secret: registration.client_secret,
        access_token: String::new(),
        refresh_token: None,
        token_type: String::new(),
        scopes,
        expires_at_ms: None,
    };
    install_token_response(&mut credential, response, false)?;
    Ok(credential)
}

fn install_token_response(
    credential: &mut McpOAuthCredential,
    response: TokenResponse,
    preserve_missing: bool,
) -> Result<(), McpOAuthError> {
    if response.access_token.trim().is_empty()
        || !response.token_type.eq_ignore_ascii_case("bearer")
    {
        return Err(McpOAuthError::TokenExchange);
    }
    credential.access_token = response.access_token;
    credential.token_type = response.token_type;
    if response.refresh_token.is_some() || !preserve_missing {
        credential.refresh_token = response.refresh_token;
    }
    if let Some(scope) = response.scope {
        credential.scopes = normalize_scopes(scope.split_ascii_whitespace().map(str::to_owned));
    }
    credential.expires_at_ms = response
        .expires_in
        .map(|seconds| epoch_ms().saturating_add(seconds.saturating_mul(1_000)));
    credential.authorization_header()?;
    Ok(())
}

fn spawn_callback_listener(
    listener: TcpListener,
    expected_path: String,
    stop: Arc<AtomicBool>,
    sender: oneshot::Sender<Result<CallbackResult, McpOAuthError>>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => match read_callback(&mut stream, &expected_path) {
                    Ok(callback) => {
                        respond(
                            &mut stream,
                            200,
                            "Authentication complete. You may close this window.",
                        );
                        let _ = sender.send(Ok(callback));
                        break;
                    }
                    Err(McpOAuthError::ProviderRejected) => {
                        respond(&mut stream, 400, "Authorization was rejected.");
                        let _ = sender.send(Err(McpOAuthError::ProviderRejected));
                        break;
                    }
                    Err(_) => respond(&mut stream, 400, "Invalid OAuth callback."),
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => {
                    let _ = sender.send(Err(McpOAuthError::CallbackListener));
                    break;
                }
            }
        }
    })
}

fn read_callback(
    stream: &mut TcpStream,
    expected_path: &str,
) -> Result<CallbackResult, McpOAuthError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| McpOAuthError::InvalidCallback)?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2_048];
    while bytes.len() <= MAX_CALLBACK_BYTES {
        let read = stream
            .read(&mut buffer)
            .map_err(|_| McpOAuthError::InvalidCallback)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    if bytes.len() > MAX_CALLBACK_BYTES {
        return Err(McpOAuthError::InvalidCallback);
    }
    let request = std::str::from_utf8(&bytes).map_err(|_| McpOAuthError::InvalidCallback)?;
    let target = request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|line| line.split_once(' '))
        .map(|(target, _)| target)
        .ok_or(McpOAuthError::InvalidCallback)?;
    parse_callback_target(target, expected_path)
}

fn parse_callback_target(
    target: &str,
    expected_path: &str,
) -> Result<CallbackResult, McpOAuthError> {
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| McpOAuthError::InvalidCallback)?;
    if url.path() != expected_path {
        return Err(McpOAuthError::InvalidCallback);
    }
    let values = url.query_pairs().collect::<BTreeMap<_, _>>();
    if values.contains_key("error") || values.contains_key("error_description") {
        return Err(McpOAuthError::ProviderRejected);
    }
    let code = values
        .get("code")
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or(McpOAuthError::InvalidCallback)?;
    let state = values
        .get("state")
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or(McpOAuthError::InvalidCallback)?;
    Ok(CallbackResult { code, state })
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn oauth_http_client() -> Result<reqwest::Client, McpOAuthError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| McpOAuthError::Discovery)
}

async fn get_optional_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: Url,
) -> Result<Option<T>, McpOAuthError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| McpOAuthError::Discovery)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(McpOAuthError::Discovery);
    }
    read_json(response, McpOAuthError::Discovery)
        .await
        .map(Some)
}

async fn post_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: Url,
    value: &serde_json::Value,
    error: McpOAuthError,
) -> Result<T, McpOAuthError> {
    let response = client
        .post(url)
        .json(value)
        .send()
        .await
        .map_err(|_| error)?;
    if !response.status().is_success() {
        return Err(error);
    }
    read_json(response, error).await
}

async fn post_form<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: Url,
    form: &BTreeMap<&str, String>,
    error: McpOAuthError,
) -> Result<T, McpOAuthError> {
    let response = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(encode_form(form))
        .send()
        .await
        .map_err(|_| error)?;
    if !response.status().is_success() {
        return Err(error);
    }
    read_json(response, error).await
}

fn encode_form(form: &BTreeMap<&str, String>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in form {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

async fn read_json<T: DeserializeOwned>(
    response: reqwest::Response,
    error: McpOAuthError,
) -> Result<T, McpOAuthError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES as u64)
    {
        return Err(error);
    }
    let bytes = response.bytes().await.map_err(|_| error)?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(error);
    }
    serde_json::from_slice(&bytes).map_err(|_| error)
}

fn protected_resource_candidates(server: &Url) -> Result<Vec<Url>, McpOAuthError> {
    well_known_candidates(server, "oauth-protected-resource")
}

fn authorization_server_candidates(server: &Url) -> Result<Vec<Url>, McpOAuthError> {
    well_known_candidates(server, "oauth-authorization-server")
}

fn well_known_candidates(server: &Url, name: &str) -> Result<Vec<Url>, McpOAuthError> {
    let mut origin = server.clone();
    origin.set_path("/");
    origin.set_query(None);
    let path = server.path().trim_end_matches('/');
    let mut candidates = Vec::new();
    if !path.is_empty() && path != "/" {
        candidates.push(
            origin
                .join(&format!(".well-known/{name}{path}"))
                .map_err(|_| McpOAuthError::InvalidConfiguration)?,
        );
    }
    candidates.push(
        origin
            .join(&format!(".well-known/{name}"))
            .map_err(|_| McpOAuthError::InvalidConfiguration)?,
    );
    Ok(candidates)
}

fn validate_endpoint(value: &str) -> Result<Url, McpOAuthError> {
    let url = Url::parse(value).map_err(|_| McpOAuthError::InvalidConfiguration)?;
    let loopback = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
    if (url.scheme() != "https" && !loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(McpOAuthError::InvalidConfiguration);
    }
    Ok(url)
}

fn selected_scopes(
    requested: &[String],
    supported: &[String],
) -> Result<Vec<String>, McpOAuthError> {
    let scopes = normalize_scopes(requested.iter().cloned());
    if !supported.is_empty() && scopes.iter().any(|scope| !supported.contains(scope)) {
        return Err(McpOAuthError::InvalidConfiguration);
    }
    Ok(scopes)
}

fn normalize_scopes(values: impl IntoIterator<Item = String>) -> Vec<String> {
    values
        .into_iter()
        .map(|scope| scope.trim().to_owned())
        .filter(|scope| !scope.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn random_urlsafe(uuid_count: usize) -> String {
    let mut bytes = Vec::with_capacity(uuid_count.saturating_mul(16));
    for _ in 0..uuid_count {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    URL_SAFE_NO_PAD.encode(bytes)
}

fn epoch_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex, OnceLock},
        thread::JoinHandle,
    };

    use super::*;

    fn network_test_lock() -> &'static tokio::sync::Mutex<()> {
        static NETWORK_TEST: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        NETWORK_TEST.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    struct OAuthFixture {
        base_url: String,
        requests: Arc<Mutex<Vec<(String, String)>>>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl OAuthFixture {
        fn start() -> Self {
            Self::start_with_protected_metadata(true)
        }

        fn start_with_protected_metadata(protected_metadata: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
            listener.set_nonblocking(true).expect("nonblocking");
            let base_url = format!("http://{}", listener.local_addr().expect("address"));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let server_base = base_url.clone();
            let server_requests = Arc::clone(&requests);
            let server_stop = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                while !server_stop.load(Ordering::Acquire) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(_) => break,
                    };
                    // Accepted sockets can inherit nonblocking mode on
                    // Windows. The fixture performs bounded blocking reads,
                    // so normalize the child socket before parsing HTTP.
                    stream
                        .set_nonblocking(false)
                        .expect("blocking fixture stream");
                    let (method, target, body) = read_fixture_request(&mut stream);
                    server_requests
                        .lock()
                        .expect("requests")
                        .push((target.clone(), body.clone()));
                    let (status, response) = match (method.as_str(), target.as_str()) {
                        ("GET", "/.well-known/oauth-protected-resource/mcp")
                            if !protected_metadata =>
                        {
                            (404, "{}".into())
                        }
                        ("GET", "/.well-known/oauth-protected-resource/mcp") => (
                            200,
                            serde_json::json!({
                                "authorization_servers": [format!("{server_base}/issuer")],
                                "scopes_supported": ["mcp.read", "mcp.write"]
                            })
                            .to_string(),
                        ),
                        (
                            "GET",
                            "/.well-known/oauth-authorization-server/issuer"
                            | "/.well-known/oauth-authorization-server/mcp",
                        ) => (
                            200,
                            serde_json::json!({
                                "authorization_endpoint": format!("{server_base}/authorize"),
                                "token_endpoint": format!("{server_base}/token"),
                                "registration_endpoint": format!("{server_base}/register"),
                                "scopes_supported": ["mcp.read", "mcp.write"],
                                "code_challenge_methods_supported": ["S256"]
                            })
                            .to_string(),
                        ),
                        ("POST", "/register") => (
                            200,
                            serde_json::json!({ "client_id": "hachimi-test-client" }).to_string(),
                        ),
                        ("POST", "/token") if body.contains("grant_type=refresh_token") => (
                            200,
                            serde_json::json!({
                                "access_token": "refreshed-access",
                                "token_type": "Bearer",
                                "expires_in": 3600,
                                "scope": "mcp.read"
                            })
                            .to_string(),
                        ),
                        ("POST", "/token") => (
                            200,
                            serde_json::json!({
                                "access_token": "initial-access",
                                "refresh_token": "stable-refresh",
                                "token_type": "Bearer",
                                "expires_in": 1,
                                "scope": "mcp.read"
                            })
                            .to_string(),
                        ),
                        _ => (404, "{}".into()),
                    };
                    write_fixture_response(&mut stream, status, &response);
                }
            });
            Self {
                base_url,
                requests,
                stop,
                thread: Some(thread),
            }
        }

        fn mcp_url(&self) -> String {
            format!("{}/mcp", self.base_url)
        }
    }

    impl Drop for OAuthFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let result = thread.join();
                if !std::thread::panicking() {
                    result.expect("fixture thread");
                }
            }
        }
    }

    fn read_fixture_request(stream: &mut TcpStream) -> (String, String, String) {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2_048];
        let header_end = loop {
            let read = stream.read(&mut buffer).expect("read request");
            assert!(read > 0, "request ended before headers");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..header_end])
            .expect("headers")
            .to_owned();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut buffer).expect("read body");
            assert!(read > 0, "request ended before body");
            bytes.extend_from_slice(&buffer[..read]);
        }
        let request_line = headers.lines().next().expect("request line");
        let mut parts = request_line.split_ascii_whitespace();
        let method = parts.next().expect("method").to_owned();
        let target = parts.next().expect("target").to_owned();
        let body = String::from_utf8(bytes[header_end..header_end + content_length].to_vec())
            .expect("body");
        (method, target, body)
    }

    fn write_fixture_response(stream: &mut TcpStream, status: u16, body: &str) {
        let reason = if status == 200 { "OK" } else { "Not Found" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("response");
    }

    #[test]
    fn callback_requires_exact_path_code_and_state() {
        assert!(matches!(
            parse_callback_target("/wrong?code=a&state=b", "/callback/id"),
            Err(McpOAuthError::InvalidCallback)
        ));
        let callback = parse_callback_target("/callback/id?code=a%20b&state=s", "/callback/id")
            .expect("callback");
        assert_eq!(callback.code, "a b");
        assert_eq!(callback.state, "s");
        assert!(matches!(
            parse_callback_target("/callback/id?error=denied", "/callback/id"),
            Err(McpOAuthError::ProviderRejected)
        ));
    }

    #[test]
    fn credentials_are_secret_serializable_but_debug_redacted() {
        let credential = McpOAuthCredential {
            server_url: "https://example.test/mcp".into(),
            token_endpoint: "https://example.test/token".into(),
            client_id: "client".into(),
            client_secret: Some("client-secret".into()),
            access_token: "access-secret".into(),
            refresh_token: Some("refresh-secret".into()),
            token_type: "Bearer".into(),
            scopes: vec!["read".into()],
            expires_at_ms: None,
        };
        let debug = format!("{credential:?}");
        assert!(!debug.contains("access-secret"));
        assert!(!debug.contains("refresh-secret"));
        let encoded = credential.to_secret_json().expect("encode");
        assert!(encoded.contains("access-secret"));
        assert_eq!(
            McpOAuthCredential::from_secret_json(&encoded)
                .expect("decode")
                .authorization_header()
                .expect("header"),
            "Bearer access-secret"
        );
        let mut expired_without_refresh = credential;
        expired_without_refresh.expires_at_ms = Some(0);
        expired_without_refresh.refresh_token = None;
        assert!(!expired_without_refresh.can_authorize(epoch_ms()));
    }

    #[test]
    fn well_known_paths_follow_resource_path_before_root() {
        let url = Url::parse("https://example.test/mcp/v1").expect("URL");
        let candidates = protected_resource_candidates(&url).expect("candidates");
        assert_eq!(
            candidates[0].as_str(),
            "https://example.test/.well-known/oauth-protected-resource/mcp/v1"
        );
        assert_eq!(
            candidates[1].as_str(),
            "https://example.test/.well-known/oauth-protected-resource"
        );
    }

    #[tokio::test]
    async fn login_uses_discovery_dcr_pkce_exact_callback_and_refresh() {
        let _network = network_test_lock().lock().await;
        let fixture = OAuthFixture::start();
        let server_url = fixture.mcp_url();
        let login = start_mcp_oauth_login(&server_url, &["mcp.read".into()], Some(5))
            .await
            .expect("start login");
        let authorization = Url::parse(login.authorization_url()).expect("authorization URL");
        let query = authorization.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            query.get("response_type").map(|value| value.as_ref()),
            Some("code")
        );
        assert_eq!(
            query
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
        assert_eq!(
            query.get("resource").map(|value| value.as_ref()),
            Some(server_url.as_str())
        );
        assert_eq!(
            query.get("scope").map(|value| value.as_ref()),
            Some("mcp.read")
        );
        let redirect_uri = query.get("redirect_uri").expect("redirect URI").to_string();
        let state = query.get("state").expect("state").to_string();
        let callback = format!("{redirect_uri}?code=provider-code&state={state}");
        let callback_response = reqwest::get(callback).await.expect("callback");
        assert!(callback_response.status().is_success());
        let credential = login.wait().await.expect("credential");
        assert_eq!(
            credential.authorization_header().expect("authorization"),
            "Bearer initial-access"
        );
        let mut expired = credential;
        expired.expires_at_ms = Some(0);
        let refreshed = refresh_mcp_oauth_credential(expired)
            .await
            .expect("refresh");
        assert_eq!(
            refreshed.authorization_header().expect("authorization"),
            "Bearer refreshed-access"
        );
        assert_eq!(refreshed.refresh_token.as_deref(), Some("stable-refresh"));
        let requests = fixture.requests.lock().expect("requests");
        let registration = requests
            .iter()
            .find(|(path, _)| path == "/register")
            .expect("registration");
        assert!(registration.1.contains("redirect_uris"));
        assert!(!registration.1.contains("initial-access"));
        let exchange = requests
            .iter()
            .find(|(path, body)| path == "/token" && body.contains("authorization_code"))
            .expect("exchange");
        assert!(exchange.1.contains("code_verifier="));
        assert!(exchange.1.contains("resource="));
    }

    #[tokio::test]
    async fn discovery_falls_back_to_path_scoped_authorization_server_metadata() {
        let _network = network_test_lock().lock().await;
        let fixture = OAuthFixture::start_with_protected_metadata(false);
        let discovery = discover_mcp_oauth(&fixture.mcp_url())
            .await
            .expect("discovery")
            .expect("OAuth metadata");
        assert_eq!(
            discovery.scopes_supported(),
            &["mcp.read".to_string(), "mcp.write".to_string()]
        );
        let requests = fixture.requests.lock().expect("requests");
        assert!(requests.iter().any(|(path, _)| {
            path == "/.well-known/oauth-authorization-server/issuer"
                || path == "/.well-known/oauth-authorization-server/mcp"
        }));
    }

    #[tokio::test]
    async fn login_timeout_and_explicit_cancel_close_the_callback_listener() {
        let _network = network_test_lock().lock().await;
        let fixture = OAuthFixture::start();
        let timed_out = start_mcp_oauth_login(&fixture.mcp_url(), &[], Some(1))
            .await
            .expect("start timeout login");
        assert!(matches!(
            timed_out.wait().await,
            Err(McpOAuthError::TimedOut)
        ));

        let cancelled = start_mcp_oauth_login(&fixture.mcp_url(), &[], Some(5))
            .await
            .expect("start cancellable login");
        assert!(matches!(
            cancelled.cancel().await,
            Err(McpOAuthError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn metadata_redirect_is_not_followed() {
        let _network = network_test_lock().lock().await;
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}/mcp", listener.local_addr().expect("address"));
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _ = read_fixture_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/metadata\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("response");
        });
        assert!(matches!(
            discover_mcp_oauth(&url).await,
            Err(McpOAuthError::Discovery)
        ));
        server.join().expect("server");
    }

    #[test]
    fn insecure_and_credentialed_endpoints_are_rejected() {
        for endpoint in [
            "http://example.test/mcp",
            "https://user@example.test/mcp",
            "https://example.test/mcp#fragment",
        ] {
            assert!(matches!(
                validate_endpoint(endpoint),
                Err(McpOAuthError::InvalidConfiguration)
            ));
        }
        assert!(validate_endpoint("http://127.0.0.1:1234/mcp").is_ok());
    }
}
