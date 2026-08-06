//! Production PR/MR adapters for GitHub, GitLab, Gitee and Gitea/Forgejo.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use hachimi_protocol::{
    ForgeChangeMutation, ForgeChangeRecord, ForgeChangeState, ForgeKind, ForgeRepositoryIdentity,
};
use reqwest::{Method, StatusCode, header};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

const FORGE_SECRET_SERVICE: &str = "com.hachimi.forge";
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ForgeError {
    #[error("invalid Forge configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Forge credential is not configured")]
    CredentialMissing,
    #[error("Forge credential store failed")]
    CredentialStore,
    #[error("Forge state changed after approval")]
    RevisionConflict,
    #[error("Forge source commit changed after approval")]
    CommitConflict,
    #[error("Forge source branch cannot be changed in place and no longer matches the request")]
    SourceRefConflict,
    #[error("Forge returned HTTP {status}: {message}")]
    Http { status: StatusCode, message: String },
    #[error("Forge returned an invalid response: {0}")]
    InvalidResponse(String),
    #[error("Forge mutation outcome is unknown: {0}")]
    Indeterminate(String),
    #[error("Forge query failed: {0}")]
    QueryFailed(String),
}

pub trait ForgeCredentialStore: Send + Sync {
    fn get(&self, secret_ref: &str) -> Result<Option<String>, ForgeError>;
    fn set(&self, secret_ref: &str, secret: &str) -> Result<(), ForgeError>;
    fn clear(&self, secret_ref: &str) -> Result<(), ForgeError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemForgeCredentialStore;

impl ForgeCredentialStore for SystemForgeCredentialStore {
    fn get(&self, secret_ref: &str) -> Result<Option<String>, ForgeError> {
        let entry = secret_entry(secret_ref)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ForgeError::CredentialStore),
        }
    }

    fn set(&self, secret_ref: &str, secret: &str) -> Result<(), ForgeError> {
        if secret.trim().is_empty() || secret.len() > 16_384 {
            return Err(ForgeError::InvalidConfiguration(
                "Forge token is empty or too large".into(),
            ));
        }
        secret_entry(secret_ref)?
            .set_password(secret)
            .map_err(|_| ForgeError::CredentialStore)
    }

    fn clear(&self, secret_ref: &str) -> Result<(), ForgeError> {
        match secret_entry(secret_ref)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ForgeError::CredentialStore),
        }
    }
}

fn secret_entry(secret_ref: &str) -> Result<keyring::Entry, ForgeError> {
    if secret_ref.trim().is_empty() || secret_ref.len() > 512 {
        return Err(ForgeError::InvalidConfiguration(
            "Forge secret_ref is invalid".into(),
        ));
    }
    keyring::Entry::new(FORGE_SECRET_SERVICE, secret_ref).map_err(|_| ForgeError::CredentialStore)
}

#[derive(Clone)]
pub struct ForgeClient {
    client: reqwest::Client,
    credentials: Arc<dyn ForgeCredentialStore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeMutationOutcome {
    pub record: ForgeChangeRecord,
    pub reconciled_after_unknown_response: bool,
}

impl std::fmt::Debug for ForgeClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForgeClient")
            .finish_non_exhaustive()
    }
}

impl ForgeClient {
    pub fn new(credentials: Arc<dyn ForgeCredentialStore>) -> Result<Self, ForgeError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| ForgeError::InvalidConfiguration(error.to_string()))?;
        Ok(Self {
            client,
            credentials,
        })
    }

    pub fn system() -> Result<Self, ForgeError> {
        Self::new(Arc::new(SystemForgeCredentialStore))
    }

    pub async fn query(
        &self,
        repository: &ForgeRepositoryIdentity,
        number: u64,
    ) -> Result<ForgeChangeRecord, ForgeError> {
        validate_repository(repository)?;
        let token = self.token(repository)?;
        let request = build_query(repository, number, &token)?;
        self.send(request, false).await
    }

    pub async fn mutate(
        &self,
        repository: &ForgeRepositoryIdentity,
        mutation: &ForgeChangeMutation,
        expected_revision: Option<&str>,
        expected_commit_oid: &str,
    ) -> Result<ForgeChangeRecord, ForgeError> {
        self.mutate_with_outcome(repository, mutation, expected_revision, expected_commit_oid)
            .await
            .map(|outcome| outcome.record)
    }

    pub async fn mutate_with_outcome(
        &self,
        repository: &ForgeRepositoryIdentity,
        mutation: &ForgeChangeMutation,
        expected_revision: Option<&str>,
        expected_commit_oid: &str,
    ) -> Result<ForgeMutationOutcome, ForgeError> {
        validate_repository(repository)?;
        validate_oid(expected_commit_oid)?;
        let token = self.token(repository)?;
        if let Some(number) = mutation_number(mutation) {
            let current = self
                .send(build_query(repository, number, &token)?, false)
                .await?;
            if expected_revision.is_none_or(|revision| revision != current.revision) {
                return Err(ForgeError::RevisionConflict);
            }
            if current
                .source_commit_oid
                .as_deref()
                .is_some_and(|oid| !oid.eq_ignore_ascii_case(expected_commit_oid))
            {
                return Err(ForgeError::CommitConflict);
            }
            if let ForgeChangeMutation::Update { source_ref, .. } = mutation
                && current.source_ref != *source_ref
            {
                return Err(ForgeError::SourceRefConflict);
            }
        }
        let request = build_mutation(repository, mutation, &token, expected_commit_oid)?;
        let result = if let Some(number) = mutation_number(mutation) {
            match self.send_ack(request).await {
                Ok(()) => self
                    .send(build_query(repository, number, &token)?, false)
                    .await
                    .map_err(|error| ForgeError::Indeterminate(error.to_string())),
                Err(error) => Err(error),
            }
        } else {
            self.send(request, true).await
        };
        match result {
            Err(error) if mutation_outcome_unknown(&error) => {
                match self
                    .reconcile_mutation(repository, mutation, expected_commit_oid)
                    .await
                {
                    Ok(Some(record)) => Ok(ForgeMutationOutcome {
                        record,
                        reconciled_after_unknown_response: true,
                    }),
                    Ok(None) | Err(_) => Err(error),
                }
            }
            Ok(record) => Ok(ForgeMutationOutcome {
                record,
                reconciled_after_unknown_response: false,
            }),
            Err(error) => Err(error),
        }
    }

    /// Reconciles an unknown mutation by querying remote state. A record is
    /// returned only when refs, visible fields, terminal state, and source OID
    /// prove that the original operation completed. No mutation is retried.
    pub async fn reconcile_mutation(
        &self,
        repository: &ForgeRepositoryIdentity,
        mutation: &ForgeChangeMutation,
        expected_commit_oid: &str,
    ) -> Result<Option<ForgeChangeRecord>, ForgeError> {
        validate_repository(repository)?;
        validate_oid(expected_commit_oid)?;
        let token = self.token(repository)?;
        let candidates = if let Some(number) = mutation_number(mutation) {
            vec![
                self.send(build_query(repository, number, &token)?, false)
                    .await?,
            ]
        } else {
            self.send_list(build_list(repository, &token)?).await?
        };
        Ok(candidates
            .into_iter()
            .find(|record| mutation_matches_remote(mutation, record, expected_commit_oid)))
    }

    fn token(&self, repository: &ForgeRepositoryIdentity) -> Result<String, ForgeError> {
        let reference = repository
            .secret_ref
            .as_deref()
            .ok_or(ForgeError::CredentialMissing)?;
        self.credentials
            .get(reference)?
            .filter(|value| !value.trim().is_empty())
            .ok_or(ForgeError::CredentialMissing)
    }

    async fn send(
        &self,
        request: ForgeHttpRequest,
        mutation: bool,
    ) -> Result<ForgeChangeRecord, ForgeError> {
        let mut builder = self.client.request(request.method, request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.json(&body);
        }
        let response = builder.send().await.map_err(|error| {
            if mutation {
                ForgeError::Indeterminate(error.to_string())
            } else {
                ForgeError::QueryFailed(error.to_string())
            }
        })?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| {
            if mutation {
                ForgeError::Indeterminate(error.to_string())
            } else {
                ForgeError::QueryFailed(error.to_string())
            }
        })?;
        if bytes.len() > RESPONSE_LIMIT {
            return Err(ForgeError::InvalidResponse(
                "response exceeded 2 MiB".into(),
            ));
        }
        if !status.is_success() {
            return Err(ForgeError::Http {
                status,
                message: bounded(&String::from_utf8_lossy(&bytes), 1_000),
            });
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| ForgeError::InvalidResponse(error.to_string()))?;
        parse_change(request.forge_kind, &value)
    }

    async fn send_ack(&self, request: ForgeHttpRequest) -> Result<(), ForgeError> {
        let mut builder = self.client.request(request.method, request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.json(&body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| ForgeError::Indeterminate(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ForgeError::Indeterminate(error.to_string()))?;
        if bytes.len() > RESPONSE_LIMIT {
            return Err(ForgeError::Indeterminate(
                "response exceeded 2 MiB after dispatch".into(),
            ));
        }
        if !status.is_success() {
            return Err(ForgeError::Http {
                status,
                message: bounded(&String::from_utf8_lossy(&bytes), 1_000),
            });
        }
        Ok(())
    }

    async fn send_list(
        &self,
        request: ForgeHttpRequest,
    ) -> Result<Vec<ForgeChangeRecord>, ForgeError> {
        let mut builder = self.client.request(request.method, request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| ForgeError::QueryFailed(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ForgeError::QueryFailed(error.to_string()))?;
        if bytes.len() > RESPONSE_LIMIT {
            return Err(ForgeError::InvalidResponse(
                "response exceeded 2 MiB".into(),
            ));
        }
        if !status.is_success() {
            return Err(ForgeError::Http {
                status,
                message: bounded(&String::from_utf8_lossy(&bytes), 1_000),
            });
        }
        let values: Vec<Value> = serde_json::from_slice(&bytes)
            .map_err(|error| ForgeError::InvalidResponse(error.to_string()))?;
        values
            .iter()
            .map(|value| parse_change(request.forge_kind, value))
            .collect()
    }
}

#[derive(Debug, Clone)]
struct ForgeHttpRequest {
    method: Method,
    url: Url,
    headers: BTreeMap<String, String>,
    body: Option<Value>,
    forge_kind: ForgeKind,
}

fn build_query(
    repository: &ForgeRepositoryIdentity,
    number: u64,
    token: &str,
) -> Result<ForgeHttpRequest, ForgeError> {
    if number == 0 {
        return Err(ForgeError::InvalidConfiguration(
            "PR/MR number must be positive".into(),
        ));
    }
    request(
        repository,
        Method::GET,
        change_path(repository, number, false),
        token,
        None,
    )
}

fn build_list(
    repository: &ForgeRepositoryIdentity,
    token: &str,
) -> Result<ForgeHttpRequest, ForgeError> {
    request(repository, Method::GET, list_path(repository), token, None)
}

fn build_mutation(
    repository: &ForgeRepositoryIdentity,
    mutation: &ForgeChangeMutation,
    token: &str,
    expected_commit_oid: &str,
) -> Result<ForgeHttpRequest, ForgeError> {
    let (method, path, body) = match mutation {
        ForgeChangeMutation::Create {
            title,
            body,
            source_ref,
            target_ref,
        } => (
            Method::POST,
            create_path(repository),
            create_body(repository.forge_kind, title, body, source_ref, target_ref)?,
        ),
        ForgeChangeMutation::Update {
            number,
            title,
            body,
            source_ref,
            target_ref,
        } => (
            update_method(repository.forge_kind),
            change_path(repository, *number, false),
            update_body(repository.forge_kind, title, body, source_ref, target_ref)?,
        ),
        ForgeChangeMutation::Close { number } => (
            update_method(repository.forge_kind),
            change_path(repository, *number, false),
            close_body(repository.forge_kind),
        ),
        ForgeChangeMutation::Merge {
            number,
            merge_title,
            merge_message,
        } => (
            merge_method(repository.forge_kind),
            change_path(repository, *number, true),
            merge_body(
                repository.forge_kind,
                merge_title.as_deref(),
                merge_message.as_deref(),
                expected_commit_oid,
            ),
        ),
    };
    request(repository, method, path, token, Some(body))
}

fn request(
    repository: &ForgeRepositoryIdentity,
    method: Method,
    path: String,
    token: &str,
    body: Option<Value>,
) -> Result<ForgeHttpRequest, ForgeError> {
    let base = Url::parse(&repository.api_base_url)
        .map_err(|error| ForgeError::InvalidConfiguration(error.to_string()))?;
    let url = base
        .join(path.trim_start_matches('/'))
        .map_err(|error| ForgeError::InvalidConfiguration(error.to_string()))?;
    let mut headers = BTreeMap::new();
    headers.insert(
        header::ACCEPT.as_str().into(),
        accept(repository.forge_kind).into(),
    );
    headers.insert(header::USER_AGENT.as_str().into(), "Hachimi/0.3".into());
    match repository.forge_kind {
        ForgeKind::GitHub => {
            headers.insert(
                header::AUTHORIZATION.as_str().into(),
                format!("Bearer {token}"),
            );
            headers.insert("x-github-api-version".into(), "2022-11-28".into());
        }
        ForgeKind::GitLab => {
            headers.insert("private-token".into(), token.into());
        }
        ForgeKind::Gitee | ForgeKind::GiteaForgejo => {
            headers.insert(
                header::AUTHORIZATION.as_str().into(),
                format!("token {token}"),
            );
        }
        ForgeKind::Unknown => {
            return Err(ForgeError::InvalidConfiguration(
                "unknown remotes support Git push only".into(),
            ));
        }
    }
    Ok(ForgeHttpRequest {
        method,
        url,
        headers,
        body,
        forge_kind: repository.forge_kind,
    })
}

fn change_path(repository: &ForgeRepositoryIdentity, number: u64, merge: bool) -> String {
    let owner = encode_path(&repository.owner);
    let repo = encode_path(&repository.repository);
    match repository.forge_kind {
        ForgeKind::GitHub => format!(
            "repos/{owner}/{repo}/pulls/{number}{}",
            if merge { "/merge" } else { "" }
        ),
        ForgeKind::GitLab => format!(
            "projects/{}/merge_requests/{number}{}",
            encode_path(&format!("{}/{}", repository.owner, repository.repository)),
            if merge { "/merge" } else { "" }
        ),
        ForgeKind::Gitee => format!(
            "repos/{owner}/{repo}/pulls/{number}{}",
            if merge { "/merge" } else { "" }
        ),
        ForgeKind::GiteaForgejo => format!(
            "repos/{owner}/{repo}/pulls/{number}{}",
            if merge { "/merge" } else { "" }
        ),
        ForgeKind::Unknown => String::new(),
    }
}

fn create_path(repository: &ForgeRepositoryIdentity) -> String {
    let owner = encode_path(&repository.owner);
    let repo = encode_path(&repository.repository);
    match repository.forge_kind {
        ForgeKind::GitHub | ForgeKind::Gitee | ForgeKind::GiteaForgejo => {
            format!("repos/{owner}/{repo}/pulls")
        }
        ForgeKind::GitLab => format!(
            "projects/{}/merge_requests",
            encode_path(&format!("{}/{}", repository.owner, repository.repository))
        ),
        ForgeKind::Unknown => String::new(),
    }
}

fn list_path(repository: &ForgeRepositoryIdentity) -> String {
    let separator = if repository.forge_kind == ForgeKind::GitLab {
        "?scope=all&state=all&per_page=100"
    } else if repository.forge_kind == ForgeKind::GiteaForgejo {
        "?state=all&limit=100"
    } else {
        "?state=all&per_page=100"
    };
    format!("{}{separator}", create_path(repository))
}

fn create_body(
    kind: ForgeKind,
    title: &str,
    body: &str,
    source_ref: &str,
    target_ref: &str,
) -> Result<Value, ForgeError> {
    validate_text(title, body, source_ref, target_ref)?;
    Ok(match kind {
        ForgeKind::GitLab => json!({
            "title": title, "description": body,
            "source_branch": source_ref, "target_branch": target_ref
        }),
        _ => json!({"title": title, "body": body, "head": source_ref, "base": target_ref}),
    })
}

fn update_body(
    kind: ForgeKind,
    title: &str,
    body: &str,
    source_ref: &str,
    target_ref: &str,
) -> Result<Value, ForgeError> {
    validate_text(title, body, source_ref, target_ref)?;
    Ok(match kind {
        ForgeKind::GitLab => json!({
            "title": title, "description": body,
            "target_branch": target_ref
        }),
        _ => json!({"title": title, "body": body, "base": target_ref}),
    })
}

fn close_body(kind: ForgeKind) -> Value {
    match kind {
        ForgeKind::GitLab => json!({"state_event": "close"}),
        _ => json!({"state": "closed"}),
    }
}

fn merge_body(
    kind: ForgeKind,
    title: Option<&str>,
    message: Option<&str>,
    expected_commit_oid: &str,
) -> Value {
    match kind {
        ForgeKind::GitHub => json!({
            "commit_title": title,
            "commit_message": message,
            "sha": expected_commit_oid,
        }),
        ForgeKind::GitLab => json!({
            "merge_commit_message": message,
            "sha": expected_commit_oid,
        }),
        ForgeKind::Gitee => json!({
            "title": title,
            "merge_message": message,
            "sha": expected_commit_oid,
        }),
        ForgeKind::GiteaForgejo => {
            json!({
                "Do": "merge",
                "MergeTitleField": title,
                "MergeMessageField": message,
                "head_commit_id": expected_commit_oid,
            })
        }
        ForgeKind::Unknown => json!({}),
    }
}

fn parse_change(kind: ForgeKind, value: &Value) -> Result<ForgeChangeRecord, ForgeError> {
    let object = value
        .as_object()
        .ok_or_else(|| ForgeError::InvalidResponse("PR/MR response must be an object".into()))?;
    let number = u64_field(object, &["number", "iid", "id"])?;
    let title = string_field(object, &["title"])?;
    let body = optional_string_field(object, &["body", "description"]).unwrap_or_default();
    let source_ref = nested_string(object, &["head", "ref"])
        .or_else(|| optional_string_field(object, &["source_branch"]))
        .ok_or_else(|| ForgeError::InvalidResponse("source ref is missing".into()))?;
    let target_ref = nested_string(object, &["base", "ref"])
        .or_else(|| optional_string_field(object, &["target_branch"]))
        .ok_or_else(|| ForgeError::InvalidResponse("target ref is missing".into()))?;
    let source_commit_oid = nested_string(object, &["head", "sha"])
        .or_else(|| optional_string_field(object, &["sha", "head_sha"]));
    if source_commit_oid
        .as_ref()
        .is_some_and(|oid| validate_oid(oid).is_err())
    {
        return Err(ForgeError::InvalidResponse(
            "source commit OID is invalid".into(),
        ));
    }
    let state_text = optional_string_field(object, &["state", "status"])
        .unwrap_or_else(|| "unknown".into())
        .to_ascii_lowercase();
    let merged = object
        .get("merged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || state_text == "merged";
    let state = if merged {
        ForgeChangeState::Merged
    } else if matches!(state_text.as_str(), "open" | "opened") {
        ForgeChangeState::Open
    } else if matches!(state_text.as_str(), "closed" | "locked") {
        ForgeChangeState::Closed
    } else {
        ForgeChangeState::Unknown
    };
    let web_url = optional_string_field(object, &["html_url", "web_url", "url"]);
    let updated = optional_string_field(object, &["updated_at", "updated"]).unwrap_or_default();
    let revision = revision(&[
        &number.to_string(),
        &title,
        &body,
        &source_ref,
        &target_ref,
        source_commit_oid.as_deref().unwrap_or(""),
        &state_text,
        &updated,
    ]);
    Ok(ForgeChangeRecord {
        forge_kind: kind,
        number,
        title,
        body,
        source_ref,
        target_ref,
        source_commit_oid,
        state,
        web_url,
        revision,
    })
}

fn mutation_number(mutation: &ForgeChangeMutation) -> Option<u64> {
    match mutation {
        ForgeChangeMutation::Create { .. } => None,
        ForgeChangeMutation::Update { number, .. }
        | ForgeChangeMutation::Close { number }
        | ForgeChangeMutation::Merge { number, .. } => Some(*number),
    }
}

fn mutation_matches_remote(
    mutation: &ForgeChangeMutation,
    record: &ForgeChangeRecord,
    expected_commit_oid: &str,
) -> bool {
    if record
        .source_commit_oid
        .as_deref()
        .is_none_or(|oid| !oid.eq_ignore_ascii_case(expected_commit_oid))
    {
        return false;
    }
    match mutation {
        ForgeChangeMutation::Create {
            title,
            body,
            source_ref,
            target_ref,
        }
        | ForgeChangeMutation::Update {
            title,
            body,
            source_ref,
            target_ref,
            ..
        } => {
            record.title == *title
                && record.body == *body
                && record.source_ref == *source_ref
                && record.target_ref == *target_ref
                && record.state == ForgeChangeState::Open
        }
        ForgeChangeMutation::Close { .. } => record.state == ForgeChangeState::Closed,
        ForgeChangeMutation::Merge { .. } => record.state == ForgeChangeState::Merged,
    }
}

fn mutation_outcome_unknown(error: &ForgeError) -> bool {
    matches!(error, ForgeError::Indeterminate(_))
        || matches!(error, ForgeError::Http { status, .. } if status.is_server_error())
}

fn validate_repository(repository: &ForgeRepositoryIdentity) -> Result<(), ForgeError> {
    if repository.forge_kind == ForgeKind::Unknown
        || repository.owner.trim().is_empty()
        || repository.repository.trim().is_empty()
        || repository.remote_url_hash.len() != 64
        || !repository
            .remote_url_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ForgeError::InvalidConfiguration(
            "Forge repository identity is incomplete".into(),
        ));
    }
    let url = Url::parse(&repository.api_base_url)
        .map_err(|error| ForgeError::InvalidConfiguration(error.to_string()))?;
    if url.scheme() != "https" && !is_loopback_http(&url) {
        return Err(ForgeError::InvalidConfiguration(
            "Forge API must use HTTPS except loopback test servers".into(),
        ));
    }
    Ok(())
}

fn validate_text(title: &str, body: &str, source: &str, target: &str) -> Result<(), ForgeError> {
    if title.trim().is_empty()
        || title.len() > 512
        || body.len() > 64 * 1024
        || source.trim().is_empty()
        || target.trim().is_empty()
        || source.len() > 1_024
        || target.len() > 1_024
        || [title, body, source, target]
            .iter()
            .any(|value| value.contains('\0'))
    {
        return Err(ForgeError::InvalidConfiguration(
            "PR/MR fields exceed protocol bounds".into(),
        ));
    }
    Ok(())
}

fn validate_oid(value: &str) -> Result<(), ForgeError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ForgeError::InvalidConfiguration(
            "commit OID must contain 40 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn update_method(kind: ForgeKind) -> Method {
    if kind == ForgeKind::GitLab {
        Method::PUT
    } else {
        Method::PATCH
    }
}

fn merge_method(kind: ForgeKind) -> Method {
    if kind == ForgeKind::GiteaForgejo {
        Method::POST
    } else {
        Method::PUT
    }
}

fn accept(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "application/vnd.github+json",
        _ => "application/json",
    }
}

fn encode_path(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn u64_field(object: &Map<String, Value>, names: &[&str]) -> Result<u64, ForgeError> {
    names
        .iter()
        .find_map(|name| object.get(*name).and_then(Value::as_u64))
        .filter(|value| *value > 0)
        .ok_or_else(|| ForgeError::InvalidResponse("PR/MR number is missing".into()))
}

fn string_field(object: &Map<String, Value>, names: &[&str]) -> Result<String, ForgeError> {
    optional_string_field(object, names)
        .ok_or_else(|| ForgeError::InvalidResponse(format!("{} is missing", names[0])))
}

fn optional_string_field(object: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| object.get(*name).and_then(Value::as_str))
        .map(str::to_owned)
}

fn nested_string(object: &Map<String, Value>, path: &[&str]) -> Option<String> {
    let mut value = object.get(path[0])?;
    for segment in &path[1..] {
        value = value.get(*segment)?;
    }
    value.as_str().map(str::to_owned)
}

fn revision(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_loopback_http(url: &Url) -> bool {
    url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository(kind: ForgeKind) -> ForgeRepositoryIdentity {
        ForgeRepositoryIdentity {
            forge_kind: kind,
            api_base_url: "https://forge.example.test/api/v1/".into(),
            owner: "team".into(),
            repository: "repo".into(),
            remote_url_hash: "a".repeat(64),
            secret_ref: Some("test-token".into()),
        }
    }

    #[test]
    fn adapters_build_platform_specific_create_and_merge_requests() {
        let create = ForgeChangeMutation::Create {
            title: "Change".into(),
            body: "Body".into(),
            source_ref: "feature".into(),
            target_ref: "main".into(),
        };
        let github = build_mutation(
            &repository(ForgeKind::GitHub),
            &create,
            "secret",
            &"a".repeat(40),
        )
        .expect("github");
        assert_eq!(github.method, Method::POST);
        assert!(github.url.path().ends_with("/repos/team/repo/pulls"));
        assert_eq!(github.body.expect("body")["head"], "feature");

        let gitlab = build_mutation(
            &repository(ForgeKind::GitLab),
            &create,
            "secret",
            &"a".repeat(40),
        )
        .expect("gitlab");
        assert!(
            gitlab
                .url
                .path()
                .contains("projects/team%2Frepo/merge_requests")
        );
        assert_eq!(gitlab.body.expect("body")["source_branch"], "feature");

        let merge = ForgeChangeMutation::Merge {
            number: 7,
            merge_title: None,
            merge_message: Some("merge".into()),
        };
        let gitea = build_mutation(
            &repository(ForgeKind::GiteaForgejo),
            &merge,
            "secret",
            &"a".repeat(40),
        )
        .expect("gitea");
        assert_eq!(gitea.method, Method::POST);
        assert!(gitea.url.path().ends_with("/pulls/7/merge"));
        assert_eq!(
            gitea.body.expect("merge body")["head_commit_id"],
            "a".repeat(40)
        );
    }

    #[test]
    fn update_treats_source_ref_as_a_precondition_and_only_mutates_supported_fields() {
        let github = update_body(ForgeKind::GitHub, "Title", "Body", "feature", "main")
            .expect("github update");
        assert_eq!(
            github,
            json!({"title": "Title", "body": "Body", "base": "main"})
        );
        let gitlab = update_body(ForgeKind::GitLab, "Title", "Body", "feature", "main")
            .expect("gitlab update");
        assert_eq!(
            gitlab,
            json!({"title": "Title", "description": "Body", "target_branch": "main"})
        );
    }

    #[test]
    fn response_parsing_normalizes_pr_and_mr_shapes() {
        let github = parse_change(
            ForgeKind::GitHub,
            &json!({
                "number": 3, "title": "Change", "body": "Body", "state": "open",
                "head": {"ref": "feature", "sha": "a".repeat(40)},
                "base": {"ref": "main"}, "html_url": "https://example.test/pr/3",
                "updated_at": "2026-07-30T00:00:00Z"
            }),
        )
        .expect("github");
        assert_eq!(github.state, ForgeChangeState::Open);
        assert_eq!(github.source_ref, "feature");

        let gitlab = parse_change(
            ForgeKind::GitLab,
            &json!({
                "iid": 8, "title": "MR", "description": "Body", "state": "merged",
                "source_branch": "feature", "target_branch": "main", "sha": "b".repeat(40),
                "web_url": "https://example.test/mr/8", "updated_at": "2026-07-30"
            }),
        )
        .expect("gitlab");
        assert_eq!(gitlab.state, ForgeChangeState::Merged);
        assert_ne!(github.revision, gitlab.revision);
    }

    #[test]
    fn reconciliation_requires_exact_fields_state_and_source_oid() {
        let record = ForgeChangeRecord {
            forge_kind: ForgeKind::GitHub,
            number: 9,
            title: "Change".into(),
            body: "Body".into(),
            source_ref: "feature".into(),
            target_ref: "main".into(),
            source_commit_oid: Some("a".repeat(40)),
            state: ForgeChangeState::Open,
            web_url: None,
            revision: "revision".into(),
        };
        let create = ForgeChangeMutation::Create {
            title: "Change".into(),
            body: "Body".into(),
            source_ref: "feature".into(),
            target_ref: "main".into(),
        };
        assert!(mutation_matches_remote(&create, &record, &"a".repeat(40)));
        assert!(!mutation_matches_remote(&create, &record, &"b".repeat(40)));
        let mut changed = record.clone();
        changed.body = "remote drift".into();
        assert!(!mutation_matches_remote(&create, &changed, &"a".repeat(40)));

        let mut closed = record.clone();
        closed.state = ForgeChangeState::Closed;
        assert!(mutation_matches_remote(
            &ForgeChangeMutation::Close { number: 9 },
            &closed,
            &"a".repeat(40)
        ));
        assert!(!mutation_matches_remote(
            &ForgeChangeMutation::Merge {
                number: 9,
                merge_title: None,
                merge_message: None,
            },
            &closed,
            &"a".repeat(40)
        ));
    }

    #[test]
    fn reconciliation_lists_bounded_history_per_forge() {
        assert!(
            list_path(&repository(ForgeKind::GitHub)).ends_with("/pulls?state=all&per_page=100")
        );
        assert!(
            list_path(&repository(ForgeKind::GitLab))
                .ends_with("/merge_requests?scope=all&state=all&per_page=100")
        );
        assert!(
            list_path(&repository(ForgeKind::GiteaForgejo)).ends_with("/pulls?state=all&limit=100")
        );
    }
}
