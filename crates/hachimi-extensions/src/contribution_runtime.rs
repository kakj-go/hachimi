use super::*;

pub(super) fn validate_static_surface(
    root: &Path,
    custom_ui: bool,
) -> Result<(), ExtensionHostError> {
    let (_, files) = hash_bundle(root)?;
    for relative in files {
        let extension = relative
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let allowed = if custom_ui {
            matches!(extension.as_str(), "html" | "js" | "css")
        } else {
            matches!(
                extension.as_str(),
                "txt" | "json" | "css" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "woff" | "woff2"
            )
        };
        if !allowed {
            return Err(ExtensionHostError::InvalidManifest(
                "plugin static surface contains a forbidden MIME type".into(),
            ));
        }
        if custom_ui {
            let content = fs::read_to_string(root.join(relative))?;
            let lowercase = content.to_ascii_lowercase();
            if content.contains("__TAURI__")
                || content.contains("__TAURI_INTERNALS__")
                || content.contains("@tauri-apps")
                || lowercase.contains("<object")
                || lowercase.contains("<embed")
            {
                return Err(ExtensionHostError::InvalidManifest(
                    "custom UI contains a forbidden native bridge or active object".into(),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn read_json_object(path: &Path) -> Result<Value, ExtensionHostError> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    value.is_object().then_some(value).ok_or_else(|| {
        ExtensionHostError::InvalidManifest("contribution descriptor must be an object".into())
    })
}

pub(super) fn failed_contribution(
    plugin: &InstalledPlugin,
    contribution: &PluginContribution,
) -> InstalledContribution {
    let content_hash = value_hash(&json!({
        "pluginContentHash": plugin.content_hash,
        "relativePath": contribution.relative_path,
    }))
    .unwrap_or_else(|_| plugin.content_hash.clone());
    let runtime_revision = value_hash(&json!({
        "contentHash": content_hash,
        "kind": contribution.kind,
        "id": contribution.id,
    }))
    .unwrap_or_else(|_| content_hash.clone());
    InstalledContribution {
        plugin_id: plugin.manifest.id.clone(),
        contribution_id: contribution.id.clone(),
        kind: contribution.kind,
        content_hash,
        runtime_revision,
        state: ContributionRuntimeState::Failed,
        diagnostic: Some(format!("plugin_{}_load_failed", contribution.kind.as_str())),
    }
}

pub(super) fn transition_allowed(
    current: ContributionRuntimeState,
    next: ContributionRuntimeState,
) -> bool {
    use ContributionRuntimeState::{
        Active, Degraded, Disabled, Failed, Registered, Staged, Starting, Stopping, Unsupported,
    };
    current == next
        || matches!(
            (current, next),
            (Staged, Registered | Failed | Unsupported)
                | (Registered, Starting | Disabled | Failed | Unsupported)
                | (
                    Starting,
                    Active | Degraded | Disabled | Failed | Unsupported
                )
                | (Active, Starting | Stopping | Degraded | Disabled | Failed)
                | (Degraded, Starting | Stopping | Active | Disabled | Failed)
                | (Failed, Starting | Disabled)
                | (
                    Disabled,
                    Registered | Starting | Active | Failed | Unsupported
                )
                | (Stopping, Disabled | Failed)
                | (Unsupported, Registered | Disabled)
        )
}

pub(super) fn decode_installed_contribution(
    row: sqlx::sqlite::SqliteRow,
) -> Result<InstalledContribution, ExtensionHostError> {
    Ok(InstalledContribution {
        plugin_id: hachimi_protocol::PluginId::new(row.get::<String, _>("plugin_id")),
        contribution_id: row.get("contribution_id"),
        kind: parse_contribution_kind(row.get("contribution_kind"))?,
        content_hash: row.get("content_hash"),
        runtime_revision: row.get("runtime_revision"),
        state: parse_contribution_runtime_state(row.get("runtime_state"))?,
        diagnostic: row.get("diagnostic"),
    })
}

pub(super) const fn contribution_runtime_state(state: ContributionRuntimeState) -> &'static str {
    match state {
        ContributionRuntimeState::Staged => "staged",
        ContributionRuntimeState::Registered => "registered",
        ContributionRuntimeState::Starting => "starting",
        ContributionRuntimeState::Active => "active",
        ContributionRuntimeState::Degraded => "degraded",
        ContributionRuntimeState::Failed => "failed",
        ContributionRuntimeState::Disabled => "disabled",
        ContributionRuntimeState::Stopping => "stopping",
        ContributionRuntimeState::Unsupported => "unsupported",
    }
}

fn parse_contribution_runtime_state(
    state: String,
) -> Result<ContributionRuntimeState, ExtensionHostError> {
    Ok(match state.as_str() {
        "staged" => ContributionRuntimeState::Staged,
        "registered" => ContributionRuntimeState::Registered,
        "starting" => ContributionRuntimeState::Starting,
        "active" | "ready" => ContributionRuntimeState::Active,
        "degraded" | "needs_attention" => ContributionRuntimeState::Degraded,
        "failed" => ContributionRuntimeState::Failed,
        "disabled" => ContributionRuntimeState::Disabled,
        "stopping" => ContributionRuntimeState::Stopping,
        "unsupported" => ContributionRuntimeState::Unsupported,
        _ => {
            return Err(ExtensionHostError::InvalidManifest(
                "invalid contribution runtime state".into(),
            ));
        }
    })
}

fn parse_contribution_kind(kind: String) -> Result<PluginContributionKind, ExtensionHostError> {
    Ok(match kind.as_str() {
        "skill" => PluginContributionKind::Skill,
        "hook" => PluginContributionKind::Hook,
        "event_source" => PluginContributionKind::EventSource,
        "mcp" => PluginContributionKind::Mcp,
        "connector" => PluginContributionKind::Connector,
        "browser_extension" => PluginContributionKind::BrowserExtension,
        "scheduled_task_template" => PluginContributionKind::ScheduledTaskTemplate,
        "asset" => PluginContributionKind::Asset,
        "custom_ui" => PluginContributionKind::CustomUi,
        "channel" => PluginContributionKind::Channel,
        _ => {
            return Err(ExtensionHostError::InvalidManifest(
                "invalid contribution kind".into(),
            ));
        }
    })
}

pub(super) fn event_source_descriptor_valid(descriptor: &Value) -> bool {
    let Some(source_id) = descriptor.get("sourceId").and_then(Value::as_str) else {
        return false;
    };
    if source_id.is_empty() || source_id.chars().count() > 128 {
        return false;
    }
    let Some(event_types) = descriptor.get("eventTypes").and_then(Value::as_array) else {
        return false;
    };
    if event_types.is_empty() || event_types.len() > 64 {
        return false;
    }
    let mut unique = BTreeSet::new();
    event_types.iter().all(|event_type| {
        event_type.as_str().is_some_and(|event_type| {
            !event_type.is_empty() && event_type.chars().count() <= 256 && unique.insert(event_type)
        })
    })
}

pub(super) const fn plugin_status(status: PluginStatus) -> &'static str {
    match status {
        PluginStatus::Disabled => "disabled",
        PluginStatus::Enabled => "enabled",
        PluginStatus::NeedsAttention => "needs_attention",
        PluginStatus::Invalid => "invalid",
    }
}

pub(super) fn parse_plugin_status(value: String) -> Result<PluginStatus, ExtensionHostError> {
    match value.as_str() {
        "disabled" => Ok(PluginStatus::Disabled),
        "enabled" => Ok(PluginStatus::Enabled),
        "needs_attention" => Ok(PluginStatus::NeedsAttention),
        "invalid" => Ok(PluginStatus::Invalid),
        _ => Err(ExtensionHostError::InvalidManifest(
            "stored plugin status is invalid".into(),
        )),
    }
}

pub(super) const fn connector_health(health: ConnectorHealth) -> &'static str {
    match health {
        ConnectorHealth::Healthy => "healthy",
        ConnectorHealth::Revoked => "revoked",
        ConnectorHealth::SchemaDrift => "schema_drift",
        ConnectorHealth::HostIdentityDrift => "host_identity_drift",
        ConnectorHealth::ActionDrift => "action_drift",
        ConnectorHealth::RateLimited => "rate_limited",
        ConnectorHealth::Failed => "failed",
    }
}

pub(super) fn parse_connector_health(value: String) -> Result<ConnectorHealth, ExtensionHostError> {
    match value.as_str() {
        "healthy" => Ok(ConnectorHealth::Healthy),
        "revoked" => Ok(ConnectorHealth::Revoked),
        "schema_drift" => Ok(ConnectorHealth::SchemaDrift),
        "host_identity_drift" => Ok(ConnectorHealth::HostIdentityDrift),
        "action_drift" => Ok(ConnectorHealth::ActionDrift),
        "rate_limited" => Ok(ConnectorHealth::RateLimited),
        "failed" => Ok(ConnectorHealth::Failed),
        _ => Err(ExtensionHostError::ConnectorDrift),
    }
}
