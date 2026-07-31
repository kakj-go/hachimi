use std::{fmt, time::Duration};

use hachimi_forge::{ForgeClient, ForgeError};
use hachimi_protocol::{
    ApprovalId, ForgeChangeMutation, ForgeChangeRecord, ForgeKind, ForgeOperationId,
    ForgeOperationRecord, ForgeOperationStatus, ForgeRepositoryIdentity, GitPushResponse,
    GitRemoteRecord, NetworkGrant, RunId, SessionId,
};
use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};
use tokio_util::sync::CancellationToken;
use url::Url;

const GIT_REMOTE_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_MUTATION_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GitForgeHostError {
    pub(super) code: &'static str,
    pub(super) message: String,
    pub(super) indeterminate: bool,
}

impl GitForgeHostError {
    fn rejected(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            indeterminate: false,
        }
    }

    fn indeterminate(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            indeterminate: true,
        }
    }
}

impl fmt::Display for GitForgeHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.code, self.message)
    }
}

#[derive(Debug, Clone)]
pub(super) struct GitPushSpec {
    pub(super) remote_name: String,
    pub(super) expected_remote_url_hash: String,
    pub(super) source_ref: String,
    pub(super) target_ref: String,
    pub(super) expected_commit_oid: String,
}

#[derive(Debug, Clone)]
pub(super) struct ForgeMutationLedgerContext {
    pub(super) session_id: SessionId,
    pub(super) run_id: RunId,
    pub(super) run_generation: u64,
    pub(super) operation_kind: String,
    pub(super) source_ref: Option<String>,
    pub(super) target_ref: Option<String>,
    pub(super) expected_commit_oid: String,
    pub(super) expected_revision: Option<String>,
    pub(super) approval_id: Option<ApprovalId>,
    pub(super) idempotency_key: String,
    pub(super) request_hash: String,
}

#[derive(Debug, Clone)]
pub(super) struct ForgeMutationMetadata {
    pub(super) operation_kind: &'static str,
    pub(super) source_ref: Option<String>,
    pub(super) target_ref: Option<String>,
    pub(super) resource: String,
    pub(super) risk: &'static str,
}

pub(super) async fn list_git_remotes(
    host: &WorkspaceHostClient,
    cancellation: CancellationToken,
) -> Result<Vec<GitRemoteRecord>, GitForgeHostError> {
    match host
        .execute(
            WorkspaceOperation::GitRemotes,
            GIT_REMOTE_TIMEOUT,
            cancellation,
        )
        .await
    {
        Ok(WorkspaceOutput::GitRemotes { remotes }) => Ok(remotes),
        Ok(_) => Err(GitForgeHostError::rejected(
            "git_remote_protocol_mismatch",
            "Workspace Host did not return Git remotes",
        )),
        Err(error) => Err(GitForgeHostError::rejected(
            "git_remote_host_failed",
            error.message,
        )),
    }
}

pub(super) async fn project_remote_network_grant(
    host: &WorkspaceHostClient,
    cancellation: CancellationToken,
) -> Result<NetworkGrant, GitForgeHostError> {
    let remotes = list_git_remotes(host, cancellation).await?;
    Ok(network_grant_for_remotes(&remotes))
}

fn network_grant_for_remotes(remotes: &[GitRemoteRecord]) -> NetworkGrant {
    let mut hosts = Vec::new();
    let mut protocols = Vec::new();
    for remote in remotes {
        if local_remote_path(&remote.display_url) {
            protocols.push("file".to_owned());
            continue;
        }
        let endpoint = if let Ok(url) = Url::parse(&remote.display_url) {
            url.host_str()
                .map(|host| (url.scheme().to_ascii_lowercase(), host.to_ascii_lowercase()))
        } else {
            let without_user = remote
                .display_url
                .rsplit_once('@')
                .map_or(remote.display_url.as_str(), |(_, tail)| tail);
            without_user
                .split_once(':')
                .filter(|(host, path)| !host.is_empty() && !path.is_empty())
                .map(|(host, _)| ("ssh".to_owned(), host.to_ascii_lowercase()))
        };
        if let Some((protocol, host)) = endpoint {
            protocols.push(protocol);
            hosts.push(host);
        }
    }
    hosts.sort();
    hosts.dedup();
    protocols.sort();
    protocols.dedup();
    NetworkGrant {
        enabled: !protocols.is_empty(),
        hosts,
        protocols,
    }
}

pub(super) async fn push_git_remote(
    host: &WorkspaceHostClient,
    spec: GitPushSpec,
    cancellation: CancellationToken,
) -> Result<GitPushResponse, GitForgeHostError> {
    validate_remote_identity(&spec.remote_name, &spec.expected_remote_url_hash)?;
    validate_oid(&spec.expected_commit_oid, "git_push_invalid")?;
    if spec.source_ref.trim().is_empty() || spec.target_ref.trim().is_empty() {
        return Err(GitForgeHostError::rejected(
            "git_push_invalid",
            "Git push requires non-empty source and target refs",
        ));
    }
    match host
        .execute(
            WorkspaceOperation::GitPush {
                remote_name: spec.remote_name,
                expected_remote_url_hash: spec.expected_remote_url_hash,
                source_ref: spec.source_ref,
                target_ref: spec.target_ref,
                expected_commit_oid: spec.expected_commit_oid,
            },
            REMOTE_MUTATION_TIMEOUT,
            cancellation,
        )
        .await
    {
        Ok(WorkspaceOutput::GitPush { response }) => Ok(response),
        Ok(_) => Err(GitForgeHostError::indeterminate(
            "git_push_indeterminate",
            "Git push dispatch returned an unexpected receipt",
        )),
        Err(error) => Err(GitForgeHostError::indeterminate(
            "git_push_indeterminate",
            format!(
                "Git push may have reached the remote; refresh the remote ref before retrying ({})",
                error.message
            ),
        )),
    }
}

pub(super) async fn resolve_forge_repository(
    host: &WorkspaceHostClient,
    remote_name: &str,
    expected_remote_url_hash: &str,
    cancellation: CancellationToken,
) -> Result<ForgeRepositoryIdentity, GitForgeHostError> {
    validate_remote_identity(remote_name, expected_remote_url_hash)?;
    let remote =
        resolve_git_remote(host, remote_name, expected_remote_url_hash, cancellation).await?;
    repository_from_remote(&remote)
}

pub(super) async fn resolve_forge_repository_by_hash(
    host: &WorkspaceHostClient,
    expected_remote_url_hash: &str,
    cancellation: CancellationToken,
) -> Result<ForgeRepositoryIdentity, GitForgeHostError> {
    if expected_remote_url_hash.len() != 64
        || !expected_remote_url_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(GitForgeHostError::rejected(
            "forge_remote_invalid",
            "Git remote URL hash is invalid",
        ));
    }
    let remote = list_git_remotes(host, cancellation)
        .await?
        .into_iter()
        .find(|remote| remote.remote_url_hash == expected_remote_url_hash)
        .ok_or_else(|| {
            GitForgeHostError::rejected(
                "forge_remote_drift",
                "Git remote no longer matches the selected URL hash",
            )
        })?;
    repository_from_remote(&remote)
}

pub(super) async fn resolve_git_remote(
    host: &WorkspaceHostClient,
    remote_name: &str,
    expected_remote_url_hash: &str,
    cancellation: CancellationToken,
) -> Result<GitRemoteRecord, GitForgeHostError> {
    validate_remote_identity(remote_name, expected_remote_url_hash)?;
    let remotes = list_git_remotes(host, cancellation).await?;
    let remote = remotes
        .into_iter()
        .find(|remote| remote.name == remote_name)
        .ok_or_else(|| {
            GitForgeHostError::rejected("forge_remote_not_found", "Git remote no longer exists")
        })?;
    if remote.remote_url_hash != expected_remote_url_hash {
        return Err(GitForgeHostError::rejected(
            "forge_remote_drift",
            "Git remote URL changed before remote dispatch",
        ));
    }
    Ok(remote)
}

pub(super) fn network_grant_allows_remote(grant: &NetworkGrant, remote_url: &str) -> bool {
    if !grant.enabled {
        return false;
    }
    if local_remote_path(remote_url) {
        return grant
            .protocols
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case("file"));
    }
    let (protocol, host) = if let Ok(url) = Url::parse(remote_url) {
        let Some(host) = url.host_str() else {
            return false;
        };
        (url.scheme().to_ascii_lowercase(), host.to_ascii_lowercase())
    } else {
        let without_user = remote_url
            .rsplit_once('@')
            .map_or(remote_url, |(_, tail)| tail);
        let Some((host, path)) = without_user.split_once(':') else {
            return false;
        };
        if host.is_empty() || path.is_empty() {
            return false;
        }
        ("ssh".into(), host.to_ascii_lowercase())
    };
    let protocol_allowed = grant.protocols.is_empty()
        || grant
            .protocols
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&protocol));
    let host_allowed = grant.hosts.is_empty()
        || grant.hosts.iter().any(|allowed| {
            let allowed = allowed.to_ascii_lowercase();
            allowed == host
                || allowed
                    .strip_prefix("*.")
                    .is_some_and(|suffix| host.ends_with(&format!(".{suffix}")))
        });
    protocol_allowed && host_allowed
}

pub(super) async fn query_forge_change(
    repository: &ForgeRepositoryIdentity,
    number: u64,
) -> Result<ForgeChangeRecord, GitForgeHostError> {
    if number == 0 {
        return Err(GitForgeHostError::rejected(
            "forge_query_invalid",
            "Forge change number must be positive",
        ));
    }
    ForgeClient::system()
        .map_err(forge_error)?
        .query(repository, number)
        .await
        .map_err(forge_error)
}

pub(super) async fn mutate_forge_change(
    store: &hachimi_storage::AgentStore,
    repository: &ForgeRepositoryIdentity,
    mutation: &ForgeChangeMutation,
    context: ForgeMutationLedgerContext,
) -> Result<ForgeChangeRecord, GitForgeHostError> {
    validate_oid(&context.expected_commit_oid, "forge_commit_oid_invalid")?;
    validate_mutation_revision(mutation, context.expected_revision.as_deref())?;
    let now = now_ms();
    let operation = ForgeOperationRecord {
        id: ForgeOperationId::random(),
        session_id: context.session_id,
        run_id: Some(context.run_id),
        run_generation: Some(context.run_generation),
        operation_kind: context.operation_kind,
        repository: repository.clone(),
        source_ref: context.source_ref,
        target_ref: context.target_ref,
        commit_oid: context.expected_commit_oid.clone(),
        expected_revision: context.expected_revision.clone(),
        approval_id: context.approval_id,
        idempotency_key: context.idempotency_key,
        request_hash: context.request_hash,
        status: ForgeOperationStatus::Claimed,
        result: None,
        error_code: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    let operation = store
        .claim_forge_operation(&operation)
        .await
        .map_err(|error| {
            GitForgeHostError::rejected(
                "forge_operation_claim_failed",
                format!("Forge operation ledger claim failed: {error}"),
            )
        })?;
    if operation.status == ForgeOperationStatus::Confirmed {
        return operation.result.ok_or_else(|| {
            GitForgeHostError::indeterminate(
                "forge_receipt_missing",
                "confirmed Forge operation is missing its durable result",
            )
        });
    }
    if operation.status != ForgeOperationStatus::Claimed {
        return Err(GitForgeHostError::indeterminate(
            "forge_operation_indeterminate",
            "the previous Forge mutation is not safe to repeat; query the PR/MR first",
        ));
    }
    store
        .update_forge_operation(
            &operation.id,
            ForgeOperationStatus::Claimed,
            ForgeOperationStatus::Dispatched,
            None,
            None,
            now_ms(),
        )
        .await
        .map_err(|error| {
            GitForgeHostError::rejected(
                "forge_dispatch_ledger_failed",
                format!("Forge dispatch ledger update failed: {error}"),
            )
        })?;
    let result = match ForgeClient::system() {
        Ok(client) => {
            client
                .mutate(
                    repository,
                    mutation,
                    context.expected_revision.as_deref(),
                    &context.expected_commit_oid,
                )
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(result) => {
            store
                .update_forge_operation(
                    &operation.id,
                    ForgeOperationStatus::Dispatched,
                    ForgeOperationStatus::Confirmed,
                    Some(&result),
                    None,
                    now_ms(),
                )
                .await
                .map_err(|error| {
                    GitForgeHostError::indeterminate(
                        "forge_receipt_store_failed",
                        format!("Forge succeeded but its receipt could not be stored: {error}"),
                    )
                })?;
            Ok(result)
        }
        Err(error) if forge_error_is_indeterminate(&error) => {
            let _ = store
                .update_forge_operation(
                    &operation.id,
                    ForgeOperationStatus::Dispatched,
                    ForgeOperationStatus::Indeterminate,
                    None,
                    Some("forge_unknown_outcome"),
                    now_ms(),
                )
                .await;
            Err(GitForgeHostError::indeterminate(
                "forge_operation_indeterminate",
                format!("Forge mutation outcome is unknown; query before retrying ({error})"),
            ))
        }
        Err(error) => {
            store
                .update_forge_operation(
                    &operation.id,
                    ForgeOperationStatus::Dispatched,
                    ForgeOperationStatus::Failed,
                    None,
                    Some("forge_rejected"),
                    now_ms(),
                )
                .await
                .map_err(|store_error| {
                    GitForgeHostError::rejected(
                        "forge_failure_store_failed",
                        format!("Forge rejection could not be recorded: {store_error}"),
                    )
                })?;
            Err(forge_error(error))
        }
    }
}

pub(super) fn mutation_metadata(
    mutation: &ForgeChangeMutation,
    repository: &ForgeRepositoryIdentity,
) -> ForgeMutationMetadata {
    let repo = format!(
        "{}:{}/{}",
        repository.forge_kind.as_str(),
        repository.owner,
        repository.repository
    );
    match mutation {
        ForgeChangeMutation::Create {
            source_ref,
            target_ref,
            ..
        } => ForgeMutationMetadata {
            operation_kind: "forge.change.create",
            source_ref: Some(source_ref.clone()),
            target_ref: Some(target_ref.clone()),
            resource: repo,
            risk: "Create a PR/MR on an external Forge",
        },
        ForgeChangeMutation::Update {
            number,
            source_ref,
            target_ref,
            ..
        } => ForgeMutationMetadata {
            operation_kind: "forge.change.update",
            source_ref: Some(source_ref.clone()),
            target_ref: Some(target_ref.clone()),
            resource: format!("{repo}#{number}"),
            risk: "Update an external PR/MR",
        },
        ForgeChangeMutation::Close { number } => ForgeMutationMetadata {
            operation_kind: "forge.change.close",
            source_ref: None,
            target_ref: None,
            resource: format!("{repo}#{number}"),
            risk: "Close an external PR/MR",
        },
        ForgeChangeMutation::Merge { number, .. } => ForgeMutationMetadata {
            operation_kind: "forge.change.merge",
            source_ref: None,
            target_ref: None,
            resource: format!("{repo}#{number}"),
            risk: "High risk: merge an external PR/MR into its target branch",
        },
    }
}

fn repository_from_remote(
    remote: &GitRemoteRecord,
) -> Result<ForgeRepositoryIdentity, GitForgeHostError> {
    if remote.forge_kind == ForgeKind::Unknown {
        return Err(GitForgeHostError::rejected(
            "forge_remote_unsupported",
            "the selected Git remote is not a supported Forge",
        ));
    }
    let (host, path) = parse_remote_location(&remote.display_url)?;
    let mut parts = path
        .trim_matches('/')
        .trim_end_matches(".git")
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let repository = parts.pop().ok_or_else(|| {
        GitForgeHostError::rejected("forge_remote_invalid", "remote repository name is missing")
    })?;
    let owner = parts.join("/");
    if owner.is_empty()
        || repository.is_empty()
        || parts.iter().any(|part| part == "." || part == "..")
    {
        return Err(GitForgeHostError::rejected(
            "forge_remote_invalid",
            "remote owner or repository is invalid",
        ));
    }
    let api_base_url = match remote.forge_kind {
        ForgeKind::GitHub => "https://api.github.com/".to_owned(),
        ForgeKind::GitLab => format!("https://{host}/api/v4/"),
        ForgeKind::Gitee => "https://gitee.com/api/v5/".to_owned(),
        ForgeKind::GiteaForgejo => loopback_gitea_api_base(&remote.display_url)
            .unwrap_or_else(|| format!("https://{host}/api/v1/")),
        ForgeKind::Unknown => unreachable!(),
    };
    Ok(ForgeRepositoryIdentity {
        forge_kind: remote.forge_kind,
        api_base_url,
        owner,
        repository,
        remote_url_hash: remote.remote_url_hash.clone(),
        secret_ref: Some(format!(
            "forge:{}:{}",
            forge_secret_kind(remote.forge_kind),
            remote.remote_url_hash.chars().take(24).collect::<String>()
        )),
    })
}

fn loopback_gitea_api_base(remote_url: &str) -> Option<String> {
    let url = Url::parse(remote_url).ok()?;
    if url.scheme() != "http" || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    {
        return None;
    }
    Some(format!("{}/api/v1/", url.origin().ascii_serialization()))
}

fn local_remote_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if Url::parse(value).is_ok_and(|url| url.scheme() == "file") {
        return true;
    }
    let bytes = value.as_bytes();
    (bytes.len() >= 3 && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'))
        || value.starts_with("\\\\")
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || (!value.contains("://") && !value.contains('@') && !value.contains(':'))
}

fn forge_secret_kind(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "git_hub",
        ForgeKind::GitLab => "git_lab",
        ForgeKind::Gitee => "gitee",
        ForgeKind::GiteaForgejo => "gitea_forgejo",
        ForgeKind::Unknown => "unknown",
    }
}

fn parse_remote_location(value: &str) -> Result<(String, String), GitForgeHostError> {
    if let Ok(url) = Url::parse(value) {
        let host = url.host_str().ok_or_else(|| {
            GitForgeHostError::rejected("forge_remote_invalid", "remote host is missing")
        })?;
        let authority = url
            .port()
            .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"));
        return Ok((authority, url.path().to_owned()));
    }
    let without_user = value.rsplit_once('@').map_or(value, |(_, tail)| tail);
    let (host, path) = without_user.split_once(':').ok_or_else(|| {
        GitForgeHostError::rejected(
            "forge_remote_invalid",
            "remote must be an HTTP(S), SSH, or SCP-style URL",
        )
    })?;
    if host.trim().is_empty() || path.trim().is_empty() {
        return Err(GitForgeHostError::rejected(
            "forge_remote_invalid",
            "remote host or path is missing",
        ));
    }
    Ok((host.to_owned(), path.to_owned()))
}

fn validate_remote_identity(name: &str, hash: &str) -> Result<(), GitForgeHostError> {
    if name.trim().is_empty()
        || hash.len() != 64
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(GitForgeHostError::rejected(
            "git_remote_identity_invalid",
            "remote name or URL hash is invalid",
        ));
    }
    Ok(())
}

fn validate_oid(oid: &str, code: &'static str) -> Result<(), GitForgeHostError> {
    if oid.len() == 40 && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(GitForgeHostError::rejected(
            code,
            "the exact 40-character source commit OID is required",
        ))
    }
}

fn validate_mutation_revision(
    mutation: &ForgeChangeMutation,
    expected_revision: Option<&str>,
) -> Result<(), GitForgeHostError> {
    match mutation {
        ForgeChangeMutation::Create { .. } if expected_revision.is_none() => Ok(()),
        ForgeChangeMutation::Create { .. } => Err(GitForgeHostError::rejected(
            "forge_revision_invalid",
            "create requires expectedRevision=null",
        )),
        _ if expected_revision.is_some_and(|revision| !revision.trim().is_empty()) => Ok(()),
        _ => Err(GitForgeHostError::rejected(
            "forge_revision_required",
            "update, close, and merge require the current queried revision",
        )),
    }
}

fn forge_error_is_indeterminate(error: &ForgeError) -> bool {
    matches!(error, ForgeError::Indeterminate(_))
        || matches!(error, ForgeError::Http { status, .. } if status.is_server_error())
}

fn forge_error(error: ForgeError) -> GitForgeHostError {
    if forge_error_is_indeterminate(&error) {
        return GitForgeHostError::indeterminate(
            "forge_operation_indeterminate",
            error.to_string(),
        );
    }
    let code = match &error {
        ForgeError::CredentialMissing | ForgeError::CredentialStore => "forge_credential_failed",
        ForgeError::RevisionConflict => "forge_revision_conflict",
        ForgeError::CommitConflict => "forge_commit_conflict",
        ForgeError::SourceRefConflict => "forge_source_ref_conflict",
        ForgeError::Http { .. } => "forge_http_failed",
        ForgeError::QueryFailed(_) => "forge_query_failed",
        ForgeError::InvalidConfiguration(_) | ForgeError::InvalidResponse(_) => {
            "forge_protocol_failed"
        }
        ForgeError::Indeterminate(_) => unreachable!(),
    };
    GitForgeHostError::rejected(code, error.to_string())
}

fn now_ms() -> i64 {
    i64::try_from(crate::epoch_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(kind: ForgeKind, display_url: &str) -> GitRemoteRecord {
        GitRemoteRecord {
            name: "origin".into(),
            display_url: display_url.into(),
            remote_url_hash: "a".repeat(64),
            forge_kind: kind,
        }
    }

    #[test]
    fn forge_repository_is_derived_from_https_and_scp_remotes() {
        let github = repository_from_remote(&remote(
            ForgeKind::GitHub,
            "https://github.com/team/repository.git",
        ))
        .expect("github");
        assert_eq!(github.owner, "team");
        assert_eq!(github.repository, "repository");
        assert_eq!(github.api_base_url, "https://api.github.com/");
        assert_eq!(
            github.secret_ref.as_deref(),
            Some("forge:git_hub:aaaaaaaaaaaaaaaaaaaaaaaa")
        );

        let gitlab = repository_from_remote(&remote(
            ForgeKind::GitLab,
            "git@gitlab.example.com:group/sub/repository.git",
        ))
        .expect("gitlab");
        assert_eq!(gitlab.owner, "group/sub");
        assert_eq!(gitlab.api_base_url, "https://gitlab.example.com/api/v4/");

        let loopback = repository_from_remote(&remote(
            ForgeKind::GiteaForgejo,
            "http://127.0.0.1:43123/team/repository.git",
        ))
        .expect("loopback gitea");
        assert_eq!(loopback.api_base_url, "http://127.0.0.1:43123/api/v1/");
    }

    #[test]
    fn forge_mutations_enforce_revision_shape_before_dispatch() {
        let create = ForgeChangeMutation::Create {
            title: "title".into(),
            body: String::new(),
            source_ref: "feature".into(),
            target_ref: "main".into(),
        };
        assert!(validate_mutation_revision(&create, None).is_ok());
        assert!(validate_mutation_revision(&create, Some("revision")).is_err());
        let close = ForgeChangeMutation::Close { number: 1 };
        assert!(validate_mutation_revision(&close, None).is_err());
        assert!(validate_mutation_revision(&close, Some("revision")).is_ok());
    }

    #[test]
    fn remote_network_grant_is_exact_and_supports_explicit_wildcards() {
        let grant = NetworkGrant {
            enabled: true,
            hosts: vec!["github.com".into(), "*.example.test".into()],
            protocols: vec!["https".into(), "ssh".into()],
        };
        assert!(network_grant_allows_remote(
            &grant,
            "https://github.com/team/repository.git"
        ));
        assert!(network_grant_allows_remote(
            &grant,
            "git@forge.example.test:team/repository.git"
        ));
        assert!(!network_grant_allows_remote(
            &grant,
            "https://example.test.evil.invalid/team/repository.git"
        ));
        assert!(!network_grant_allows_remote(
            &grant,
            "git://github.com/team/repository.git"
        ));

        let local_grant = NetworkGrant {
            enabled: true,
            hosts: Vec::new(),
            protocols: vec!["file".into()],
        };
        assert!(network_grant_allows_remote(
            &local_grant,
            "C:\\temp\\repository.git"
        ));
        assert!(!network_grant_allows_remote(
            &grant,
            "C:\\temp\\repository.git"
        ));
    }

    #[test]
    fn project_remote_grant_is_exact_deduplicated_and_tracks_local_protocol() {
        let remotes = vec![
            remote(ForgeKind::GitHub, "https://github.com/team/repository.git"),
            remote(
                ForgeKind::GitLab,
                "git@forge.example.test:team/repository.git",
            ),
            remote(ForgeKind::Unknown, "C:\\workspace\\bare.git"),
            remote(ForgeKind::GitHub, "https://github.com/other/repository.git"),
        ];
        let grant = network_grant_for_remotes(&remotes);
        assert!(grant.enabled);
        assert_eq!(grant.hosts, vec!["forge.example.test", "github.com"]);
        assert_eq!(grant.protocols, vec!["file", "https", "ssh"]);
    }
}
