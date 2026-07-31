use hachimi_protocol::{
    ProviderAccountId, ProviderAccountRecord, ProviderCapabilityProbeId,
    ProviderCompatibilityProfile, ProviderCompatibilityProfileKind, ProviderEndpointId,
    ProviderEndpointRecord, ProviderProbeReport, ProviderProbeStatus,
};
use sqlx::Row;

use super::{AgentStore, AgentStoreError};

impl AgentStore {
    pub async fn list_provider_compatibility_profiles(
        &self,
    ) -> Result<Vec<ProviderCompatibilityProfile>, AgentStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM provider_compatibility_profiles ORDER BY builtin DESC, display_name, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(profile_from_row).collect()
    }

    pub async fn get_provider_compatibility_profile(
        &self,
        id: &str,
    ) -> Result<Option<ProviderCompatibilityProfile>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM provider_compatibility_profiles WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(profile_from_row).transpose()
    }

    pub async fn list_provider_endpoints(
        &self,
    ) -> Result<Vec<ProviderEndpointRecord>, AgentStoreError> {
        let rows = sqlx::query("SELECT * FROM provider_endpoints ORDER BY display_name, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(endpoint_from_row).collect()
    }

    pub async fn get_provider_endpoint(
        &self,
        id: &ProviderEndpointId,
    ) -> Result<Option<ProviderEndpointRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM provider_endpoints WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(endpoint_from_row).transpose()
    }

    pub async fn upsert_provider_endpoint(
        &self,
        endpoint: &ProviderEndpointRecord,
        expected_config_revision: Option<u64>,
    ) -> Result<ProviderEndpointRecord, AgentStoreError> {
        if self
            .get_provider_compatibility_profile(&endpoint.compatibility_profile_id)
            .await?
            .is_none()
        {
            return Err(AgentStoreError::ProviderProfileNotFound(
                endpoint.compatibility_profile_id.clone(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let existing = sqlx::query("SELECT config_revision FROM provider_endpoints WHERE id = ?")
            .bind(endpoint.id.as_str())
            .fetch_optional(&mut *transaction)
            .await?;
        let next_revision = match existing {
            Some(row) => {
                let current = u64::try_from(row.get::<i64, _>("config_revision"))
                    .map_err(|_| invalid("provider endpoint revision", -1))?;
                if expected_config_revision != Some(current) {
                    return Err(AgentStoreError::ProviderRevisionConflict);
                }
                current.saturating_add(1)
            }
            None => {
                if expected_config_revision.is_some() {
                    return Err(AgentStoreError::ProviderRevisionConflict);
                }
                1
            }
        };
        sqlx::query(
            "INSERT INTO provider_endpoints (id, display_name, base_url, compatibility_profile_id, enabled, config_revision, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, base_url = excluded.base_url, compatibility_profile_id = excluded.compatibility_profile_id, enabled = excluded.enabled, config_revision = excluded.config_revision, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(endpoint.id.as_str())
        .bind(endpoint.display_name.trim())
        .bind(endpoint.base_url.trim_end_matches('/'))
        .bind(&endpoint.compatibility_profile_id)
        .bind(endpoint.enabled)
        .bind(i64::try_from(next_revision).unwrap_or(i64::MAX))
        .bind(endpoint.created_at_ms)
        .bind(endpoint.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_provider_endpoint(&endpoint.id)
            .await?
            .ok_or_else(|| AgentStoreError::ProviderEndpointNotFound(endpoint.id.clone()))
    }

    pub async fn list_provider_accounts(
        &self,
        endpoint_id: Option<&ProviderEndpointId>,
    ) -> Result<Vec<ProviderAccountRecord>, AgentStoreError> {
        let rows = if let Some(endpoint_id) = endpoint_id {
            sqlx::query(
                "SELECT * FROM provider_accounts WHERE endpoint_id = ? ORDER BY display_name, id",
            )
            .bind(endpoint_id.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query("SELECT * FROM provider_accounts ORDER BY endpoint_id, display_name, id")
                .fetch_all(&self.pool)
                .await?
        };
        rows.iter().map(account_from_row).collect()
    }

    pub async fn get_provider_account(
        &self,
        id: &ProviderAccountId,
    ) -> Result<Option<ProviderAccountRecord>, AgentStoreError> {
        let row = sqlx::query("SELECT * FROM provider_accounts WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(account_from_row).transpose()
    }

    pub async fn upsert_provider_account(
        &self,
        account: &ProviderAccountRecord,
        expected_config_revision: Option<u64>,
    ) -> Result<ProviderAccountRecord, AgentStoreError> {
        if self
            .get_provider_endpoint(&account.endpoint_id)
            .await?
            .is_none()
        {
            return Err(AgentStoreError::ProviderEndpointNotFound(
                account.endpoint_id.clone(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let existing = sqlx::query("SELECT config_revision FROM provider_accounts WHERE id = ?")
            .bind(account.id.as_str())
            .fetch_optional(&mut *transaction)
            .await?;
        let next_revision = match existing {
            Some(row) => {
                let current = u64::try_from(row.get::<i64, _>("config_revision"))
                    .map_err(|_| invalid("provider account revision", -1))?;
                if expected_config_revision != Some(current) {
                    return Err(AgentStoreError::ProviderRevisionConflict);
                }
                current.saturating_add(1)
            }
            None => {
                if expected_config_revision.is_some() {
                    return Err(AgentStoreError::ProviderRevisionConflict);
                }
                1
            }
        };
        sqlx::query(
            "INSERT INTO provider_accounts (id, endpoint_id, display_name, secret_ref, enabled, config_revision, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET endpoint_id = excluded.endpoint_id, display_name = excluded.display_name, secret_ref = excluded.secret_ref, enabled = excluded.enabled, config_revision = excluded.config_revision, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(account.id.as_str())
        .bind(account.endpoint_id.as_str())
        .bind(account.display_name.trim())
        .bind(&account.secret_ref)
        .bind(account.enabled)
        .bind(i64::try_from(next_revision).unwrap_or(i64::MAX))
        .bind(account.created_at_ms)
        .bind(account.updated_at_ms)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_provider_account(&account.id)
            .await?
            .ok_or_else(|| AgentStoreError::ProviderAccountNotFound(account.id.clone()))
    }

    pub async fn record_provider_probe(
        &self,
        report: &ProviderProbeReport,
    ) -> Result<ProviderProbeReport, AgentStoreError> {
        if self
            .get_provider_endpoint(&report.endpoint_id)
            .await?
            .is_none()
        {
            return Err(AgentStoreError::ProviderEndpointNotFound(
                report.endpoint_id.clone(),
            ));
        }
        sqlx::query(
            "INSERT INTO provider_capability_probes (id, endpoint_id, account_id, status, protocols_json, capabilities_json, capability_revision, stable_error_code, probed_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(report.id.as_str())
        .bind(report.endpoint_id.as_str())
        .bind(report.account_id.as_ref().map(ProviderAccountId::as_str))
        .bind(report.status.as_str())
        .bind(serde_json::to_string(&report.protocols)?)
        .bind(serde_json::to_string(&report.capabilities)?)
        .bind(&report.capability_revision)
        .bind(&report.stable_error_code)
        .bind(report.probed_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(report.clone())
    }

    pub async fn latest_provider_probe(
        &self,
        endpoint_id: &ProviderEndpointId,
    ) -> Result<Option<ProviderProbeReport>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT * FROM provider_capability_probes WHERE endpoint_id = ? ORDER BY probed_at_ms DESC, id DESC LIMIT 1",
        )
        .bind(endpoint_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(probe_from_row).transpose()
    }
}

fn profile_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ProviderCompatibilityProfile, AgentStoreError> {
    let kind = row.get::<String, _>("kind");
    Ok(ProviderCompatibilityProfile {
        id: row.get("id"),
        display_name: row.get("display_name"),
        kind: ProviderCompatibilityProfileKind::parse(&kind)
            .ok_or_else(|| invalid("provider profile kind", kind))?,
        protocols: serde_json::from_str(&row.get::<String, _>("protocols_json"))?,
        profile_revision: row.get("profile_revision"),
        builtin: row.get("builtin"),
    })
}

fn endpoint_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ProviderEndpointRecord, AgentStoreError> {
    Ok(ProviderEndpointRecord {
        id: ProviderEndpointId::new(row.get::<String, _>("id")),
        display_name: row.get("display_name"),
        base_url: row.get("base_url"),
        compatibility_profile_id: row.get("compatibility_profile_id"),
        enabled: row.get("enabled"),
        config_revision: u64::try_from(row.get::<i64, _>("config_revision"))
            .map_err(|_| invalid("provider endpoint revision", -1))?,
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn account_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ProviderAccountRecord, AgentStoreError> {
    Ok(ProviderAccountRecord {
        id: ProviderAccountId::new(row.get::<String, _>("id")),
        endpoint_id: ProviderEndpointId::new(row.get::<String, _>("endpoint_id")),
        display_name: row.get("display_name"),
        secret_ref: row.get("secret_ref"),
        enabled: row.get("enabled"),
        config_revision: u64::try_from(row.get::<i64, _>("config_revision"))
            .map_err(|_| invalid("provider account revision", -1))?,
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn probe_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ProviderProbeReport, AgentStoreError> {
    let status = row.get::<String, _>("status");
    Ok(ProviderProbeReport {
        id: ProviderCapabilityProbeId::new(row.get::<String, _>("id")),
        endpoint_id: ProviderEndpointId::new(row.get::<String, _>("endpoint_id")),
        account_id: row
            .get::<Option<String>, _>("account_id")
            .map(ProviderAccountId::new),
        status: ProviderProbeStatus::parse(&status)
            .ok_or_else(|| invalid("provider probe status", status))?,
        protocols: serde_json::from_str(&row.get::<String, _>("protocols_json"))?,
        capabilities: serde_json::from_str(&row.get::<String, _>("capabilities_json"))?,
        capability_revision: row.get("capability_revision"),
        stable_error_code: row.get("stable_error_code"),
        probed_at_ms: row.get("probed_at_ms"),
    })
}

fn invalid(kind: &'static str, value: impl ToString) -> AgentStoreError {
    AgentStoreError::InvalidPersistedValue {
        kind,
        value: value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ProviderAccountId, ProviderAccountRecord, ProviderCapabilities, ProviderCapabilityProbeId,
        ProviderEndpointId, ProviderEndpointRecord, ProviderProbeReport, ProviderProbeStatus,
        ProviderProtocolKind,
    };

    use super::{AgentStore, AgentStoreError};

    #[tokio::test]
    async fn builtin_profile_and_default_registry_are_available() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let profile = store
            .get_provider_compatibility_profile("openai-strict")
            .await
            .expect("profile lookup")
            .expect("builtin profile");
        assert!(profile.builtin);
        assert_eq!(
            profile.protocols,
            vec![
                ProviderProtocolKind::ChatCompletions,
                ProviderProtocolKind::Responses,
                ProviderProtocolKind::Embeddings,
            ]
        );

        let endpoint = store
            .get_provider_endpoint(&ProviderEndpointId::new("default-openai"))
            .await
            .expect("endpoint lookup")
            .expect("default endpoint");
        let account = store
            .get_provider_account(&ProviderAccountId::new("default-openai"))
            .await
            .expect("account lookup")
            .expect("default account");
        assert_eq!(account.endpoint_id, endpoint.id);
        assert_eq!(account.secret_ref, "credential-manager:llm-api-key");
    }

    #[tokio::test]
    async fn endpoint_and_account_updates_are_revision_fenced() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let endpoint = ProviderEndpointRecord {
            id: ProviderEndpointId::new("staging"),
            display_name: "Staging".into(),
            base_url: "https://api.example.test/v1/".into(),
            compatibility_profile_id: "openai-strict".into(),
            enabled: true,
            config_revision: 0,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let inserted = store
            .upsert_provider_endpoint(&endpoint, None)
            .await
            .expect("insert endpoint");
        assert_eq!(inserted.config_revision, 1);
        assert_eq!(inserted.base_url, "https://api.example.test/v1");
        assert!(matches!(
            store.upsert_provider_endpoint(&endpoint, None).await,
            Err(AgentStoreError::ProviderRevisionConflict)
        ));
        let updated = store
            .upsert_provider_endpoint(&endpoint, Some(1))
            .await
            .expect("update endpoint");
        assert_eq!(updated.config_revision, 2);

        let account = ProviderAccountRecord {
            id: ProviderAccountId::new("staging"),
            endpoint_id: endpoint.id,
            display_name: "Staging".into(),
            secret_ref: "credential-manager:provider:staging".into(),
            enabled: true,
            config_revision: 0,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let inserted = store
            .upsert_provider_account(&account, None)
            .await
            .expect("insert account");
        assert_eq!(inserted.config_revision, 1);
        assert!(matches!(
            store.upsert_provider_account(&account, Some(9)).await,
            Err(AgentStoreError::ProviderRevisionConflict)
        ));
    }

    #[tokio::test]
    async fn capability_probe_round_trips_without_secret_material() {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let report = ProviderProbeReport {
            id: ProviderCapabilityProbeId::new("probe-1"),
            endpoint_id: ProviderEndpointId::new("default-openai"),
            account_id: Some(ProviderAccountId::new("default-openai")),
            status: ProviderProbeStatus::Succeeded,
            protocols: vec![
                ProviderProtocolKind::Responses,
                ProviderProtocolKind::Embeddings,
            ],
            capabilities: ProviderCapabilities {
                text_input: true,
                reasoning_summary: true,
                remote_compaction: true,
                embeddings: true,
                ..ProviderCapabilities::default()
            },
            capability_revision: "probe-revision".into(),
            stable_error_code: None,
            probed_at_ms: 123,
        };
        store
            .record_provider_probe(&report)
            .await
            .expect("record probe");
        assert_eq!(
            store
                .latest_provider_probe(&report.endpoint_id)
                .await
                .expect("latest probe"),
            Some(report)
        );

        let secret_like_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_accounts WHERE secret_ref LIKE '%sk-%'",
        )
        .fetch_one(store.pool())
        .await
        .expect("scan secret refs");
        assert_eq!(secret_like_rows, 0);
    }
}
