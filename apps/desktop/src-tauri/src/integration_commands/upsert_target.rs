use hachimi_protocol::IntegrationProviderId;
use sqlx::SqlitePool;

use crate::CommandError;

use super::from_i64;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct IntegrationUpsertTarget {
    pub(super) account_id: String,
    pub(super) previous_revisions: Option<(u64, u64)>,
}

pub(super) async fn resolve_integration_upsert_target(
    pool: &SqlitePool,
    requested_id: &str,
    provider_id: IntegrationProviderId,
    tenant_key: &str,
    tenant_identity_hash: &str,
    expected_config_revision: Option<u64>,
) -> Result<IntegrationUpsertTarget, CommandError> {
    let requested: Option<(i64, i64)> = sqlx::query_as(
        "SELECT credential_revision, config_revision FROM integration_provider_accounts WHERE id = ?",
    )
    .bind(requested_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| CommandError::operation("integration_account_load_failed", error))?;

    if let Some((credential_revision, config_revision)) = requested {
        if expected_config_revision != Some(from_i64(config_revision)) {
            return Err(revision_conflict());
        }
        let identity_owner: Option<String> = sqlx::query_scalar(
            "SELECT id FROM integration_provider_accounts WHERE provider_id = ? AND tenant_key = ? AND tenant_identity_hash = ?",
        )
        .bind(provider_id.as_str())
        .bind(tenant_key)
        .bind(tenant_identity_hash)
        .fetch_optional(pool)
        .await
        .map_err(|error| CommandError::operation("integration_account_load_failed", error))?;
        if identity_owner
            .as_deref()
            .is_some_and(|owner| owner != requested_id)
        {
            return Err(integration_identity_conflict());
        }
        return Ok(IntegrationUpsertTarget {
            account_id: requested_id.into(),
            previous_revisions: Some((from_i64(credential_revision), from_i64(config_revision))),
        });
    }

    if expected_config_revision.is_some() {
        return Err(revision_conflict());
    }
    let existing: Option<(String, i64, i64)> = sqlx::query_as(
        "SELECT id, credential_revision, config_revision FROM integration_provider_accounts WHERE provider_id = ? AND tenant_key = ? AND tenant_identity_hash = ?",
    )
    .bind(provider_id.as_str())
    .bind(tenant_key)
    .bind(tenant_identity_hash)
    .fetch_optional(pool)
    .await
    .map_err(|error| CommandError::operation("integration_account_load_failed", error))?;
    Ok(match existing {
        Some((account_id, credential_revision, config_revision)) => IntegrationUpsertTarget {
            account_id,
            previous_revisions: Some((from_i64(credential_revision), from_i64(config_revision))),
        },
        None => IntegrationUpsertTarget {
            account_id: requested_id.into(),
            previous_revisions: None,
        },
    })
}

pub(super) fn revision_conflict() -> CommandError {
    CommandError::new(
        "integration_revision_conflict",
        "The integration was changed by another operation.",
    )
}

fn integration_identity_conflict() -> CommandError {
    CommandError::new(
        "integration_identity_conflict",
        "This platform account is already connected. Update the existing account credentials or disconnect it before trying again.",
    )
}

pub(super) fn integration_account_store_error(error: sqlx::Error) -> CommandError {
    let identity_conflict = error.as_database_error().is_some_and(|database_error| {
        database_error.is_unique_violation()
            && database_error
                .message()
                .contains("integration_provider_accounts.provider_id")
    });
    tracing::error!(%error, "Integration account persistence failed");
    if identity_conflict {
        integration_identity_conflict()
    } else {
        CommandError::new(
            "integration_account_store_failed",
            "The platform account could not be saved. Retry the operation and check the application logs if the problem continues.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn insert_account(
        pool: &SqlitePool,
        id: &str,
        tenant_key: &str,
        tenant_identity_hash: &str,
    ) {
        sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, credential_revision, config_revision, created_at_ms, updated_at_ms) VALUES(?, 'feishu', 'Feishu', ?, ?, 'long_connection', 'needs_attention', 2, 3, 1, 1)")
            .bind(id)
            .bind(tenant_key)
            .bind(tenant_identity_hash)
            .execute(pool)
            .await
            .expect("account");
    }

    #[tokio::test]
    async fn duplicate_identity_resumes_the_existing_account() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        insert_account(store.pool(), "existing", "app-1", "hash-1").await;

        let target = resolve_integration_upsert_target(
            store.pool(),
            "new-random-id",
            IntegrationProviderId::Feishu,
            "app-1",
            "hash-1",
            None,
        )
        .await
        .expect("existing target");

        assert_eq!(
            target,
            IntegrationUpsertTarget {
                account_id: "existing".into(),
                previous_revisions: Some((2, 3)),
            }
        );
    }

    #[tokio::test]
    async fn editing_into_another_accounts_identity_is_a_business_error() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        insert_account(store.pool(), "first", "app-1", "hash-1").await;
        insert_account(store.pool(), "second", "app-2", "hash-2").await;

        let error = resolve_integration_upsert_target(
            store.pool(),
            "second",
            IntegrationProviderId::Feishu,
            "app-1",
            "hash-1",
            Some(3),
        )
        .await
        .expect_err("identity conflict");

        assert_eq!(error.code, "integration_identity_conflict");
        assert!(!error.message.contains("integration_provider_accounts"));
    }

    #[tokio::test]
    async fn unique_constraint_details_are_not_returned_to_the_ui() {
        let store = hachimi_storage::AgentStore::connect_in_memory()
            .await
            .expect("store");
        insert_account(store.pool(), "first", "app-1", "hash-1").await;
        let database_error = sqlx::query("INSERT INTO integration_provider_accounts(id, provider_id, display_name, tenant_key, tenant_identity_hash, transport, state, created_at_ms, updated_at_ms) VALUES('second', 'feishu', 'Feishu', 'app-1', 'hash-1', 'long_connection', 'draft', 1, 1)")
            .execute(store.pool())
            .await
            .expect_err("unique constraint");

        let error = integration_account_store_error(database_error);

        assert_eq!(error.code, "integration_identity_conflict");
        assert!(!error.message.contains("UNIQUE"));
        assert!(!error.message.contains("integration_provider_accounts"));
    }
}
