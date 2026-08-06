//! Connector revision snapshots shared by unattended and interactive Agent runs.

use hachimi_protocol::{
    AgentPermissionPolicy, ConnectorDriverDescriptor, ConnectorHealth, ConnectorRevisionSelection,
    ContributionRevision, HostRevisionSnapshot, PermissionProfile, ScheduleDefinition,
};

pub(super) const ENTERPRISE_ATTACHMENT_ACTION: &str = "download_attachment";
#[cfg(test)]
pub(super) const ENTERPRISE_ATTACHMENT_TOOL: &str = "enterprise.download_attachment";
pub(super) const HOST_REVISION_ATTENTION_EVENT: &str =
    "agent.host_revision_snapshot.needs_attention";

pub(super) fn snapshot_from_permission_policy(
    policy: &AgentPermissionPolicy,
    connector_revisions: &[ConnectorRevisionSelection],
) -> Option<HostRevisionSnapshot> {
    if policy.level == PermissionProfile::FullAccess {
        return None;
    }
    let connectors = connector_revisions
        .iter()
        .filter_map(|selection| {
            let rule = policy.rules.connectors.iter().find(|rule| {
                rule.account_id == selection.account_id
                    && selection.contribution_revision.action_hash.as_deref()
                        == Some(rule.contribution_revision.as_str())
            })?;
            let mut selection = selection.clone();
            selection
                .allowed_actions
                .retain(|action| rule.actions.contains(action));
            (!selection.allowed_actions.is_empty()).then_some(selection)
        })
        .collect();
    Some(HostRevisionSnapshot { connectors })
}

/// Compile an unattended policy against the currently installed Connector revisions.
/// A background run must pin the account, plugin content, and Host/Schema/Action hashes
/// before dispatch; missing or drifted state is surfaced as a launch error.
pub(super) async fn snapshot_from_current_connector_policy(
    host: &hachimi_extensions::PluginHost,
    policy: &AgentPermissionPolicy,
) -> Result<Option<HostRevisionSnapshot>, String> {
    if policy.level == PermissionProfile::FullAccess {
        return Ok(None);
    }
    let accounts = host
        .list_connector_accounts()
        .await
        .map_err(|error| format!("connector accounts unavailable: {error}"))?;
    let mut selections = Vec::with_capacity(policy.rules.connectors.len());
    for rule in &policy.rules.connectors {
        let account = accounts
            .iter()
            .find(|account| {
                account.id == rule.account_id && account.health == ConnectorHealth::Healthy
            })
            .ok_or_else(|| format!("Connector account {} is unavailable", rule.account_id))?;
        let descriptor = host
            .connector_driver_descriptor(&account.plugin_id, &account.connector_id)
            .await
            .map_err(|error| format!("Connector descriptor unavailable: {error}"))?;
        if descriptor.revision.action_hash != rule.contribution_revision {
            return Err(format!(
                "Connector action revision for {} changed",
                rule.account_id
            ));
        }
        let allowed_actions = rule
            .actions
            .iter()
            .filter(|action| {
                descriptor
                    .actions
                    .iter()
                    .any(|available| available == *action)
            })
            .cloned()
            .collect::<Vec<_>>();
        if allowed_actions.is_empty() {
            return Err(format!(
                "Connector actions for {} are unavailable",
                rule.account_id
            ));
        }
        let plugin = host
            .health_check(&account.plugin_id)
            .await
            .map_err(|error| format!("Connector plugin unavailable: {error}"))?;
        selections.push(ConnectorRevisionSelection {
            account_id: account.id.clone(),
            contribution_revision: ContributionRevision {
                plugin_id: account.plugin_id.clone(),
                contribution_id: account.connector_id.clone(),
                account_id: Some(account.id.clone()),
                content_hash: plugin.content_hash,
                host_identity_hash: Some(descriptor.revision.host_identity_hash),
                schema_hash: Some(descriptor.revision.schema_hash),
                action_hash: Some(descriptor.revision.action_hash),
            },
            allowed_actions,
        });
    }
    let snapshot = snapshot_from_permission_policy(policy, &selections)
        .ok_or_else(|| "restricted Connector policy did not produce a snapshot".to_owned())?;
    validate_connector_revision_selections(host, &snapshot.connectors)
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    Ok(Some(snapshot))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostRevisionSnapshotError {
    pub(super) code: &'static str,
    pub(super) message: String,
}

impl HostRevisionSnapshotError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            code: "agent_connector_action_not_authorized",
            message: message.into(),
        }
    }

    fn drift(message: impl Into<String>) -> Self {
        Self {
            code: "agent_connector_revision_drift",
            message: message.into(),
        }
    }
}

pub(super) fn validate_enterprise_attachment_scope(
    schedule: &ScheduleDefinition,
) -> Result<(), HostRevisionSnapshotError> {
    if !schedule
        .permission_policy
        .rules
        .connectors
        .iter()
        .any(|rule| {
            rule.actions
                .iter()
                .any(|action| action == ENTERPRISE_ATTACHMENT_ACTION)
        })
    {
        return Ok(());
    }
    if !schedule
        .host_revision_snapshot
        .connectors
        .iter()
        .any(attachment_selection_is_exact)
    {
        return Err(HostRevisionSnapshotError::unauthorized(
            "enterprise.download_attachment requires a pinned Connector account, contribution revision, and download_attachment action",
        ));
    }
    Ok(())
}

pub(super) async fn validate_connector_revision_selections(
    host: &hachimi_extensions::PluginHost,
    selections: &[ConnectorRevisionSelection],
) -> Result<(), HostRevisionSnapshotError> {
    let revisions = selections
        .iter()
        .map(|selection| selection.contribution_revision.clone())
        .collect::<Vec<_>>();
    host.verify_contribution_revisions(&revisions)
        .await
        .map_err(|error| {
            HostRevisionSnapshotError::drift(format!(
                "Connector account or contribution revision changed: {error}"
            ))
        })?;
    for selection in selections {
        let descriptor = host
            .connector_driver_descriptor(
                &selection.contribution_revision.plugin_id,
                &selection.contribution_revision.contribution_id,
            )
            .await
            .map_err(|error| {
                HostRevisionSnapshotError::drift(format!(
                    "Connector driver is unavailable or changed: {error}"
                ))
            })?;
        if !selection_matches_descriptor(selection, &descriptor) {
            return Err(HostRevisionSnapshotError::drift(
                "Connector Host identity, schema, action revision, account, or allowed action changed",
            ));
        }
    }
    Ok(())
}

pub(super) async fn authorize_enterprise_attachment_download(
    host: &hachimi_extensions::PluginHost,
    selections: &[ConnectorRevisionSelection],
    integration_account_id: &str,
) -> Result<(), HostRevisionSnapshotError> {
    let connector_account_id = host
        .enterprise_attachment_connector_account(integration_account_id)
        .await
        .map_err(|error| {
            HostRevisionSnapshotError::drift(format!(
                "enterprise attachment account binding is unavailable: {error}"
            ))
        })?
        .ok_or_else(|| {
            HostRevisionSnapshotError::unauthorized(
                "attachment integration account is not bound to a healthy Connector account",
            )
        })?;
    let selection = selections
        .iter()
        .find(|selection| {
            selection.account_id == connector_account_id
                && selection
                    .allowed_actions
                    .iter()
                    .any(|action| action == ENTERPRISE_ATTACHMENT_ACTION)
        })
        .ok_or_else(|| {
            HostRevisionSnapshotError::unauthorized(
                "attachment account or download_attachment action is outside HostRevisionSnapshot",
            )
        })?;
    if !attachment_selection_is_exact(selection) {
        return Err(HostRevisionSnapshotError::unauthorized(
            "attachment HostRevisionSnapshot does not pin the selected account",
        ));
    }
    validate_connector_revision_selections(host, std::slice::from_ref(selection)).await
}

fn attachment_selection_is_exact(selection: &ConnectorRevisionSelection) -> bool {
    selection
        .allowed_actions
        .iter()
        .any(|action| action == ENTERPRISE_ATTACHMENT_ACTION)
        && selection.contribution_revision.account_id.as_ref() == Some(&selection.account_id)
}

fn selection_matches_descriptor(
    selection: &ConnectorRevisionSelection,
    descriptor: &ConnectorDriverDescriptor,
) -> bool {
    let revision = &selection.contribution_revision;
    descriptor.revision.host_identity_hash
        == revision.host_identity_hash.clone().unwrap_or_default()
        && descriptor.revision.schema_hash == revision.schema_hash.clone().unwrap_or_default()
        && descriptor.revision.action_hash == revision.action_hash.clone().unwrap_or_default()
        && (!selection
            .allowed_actions
            .iter()
            .any(|action| action == ENTERPRISE_ATTACHMENT_ACTION)
            || revision.account_id.as_ref() == Some(&selection.account_id))
        && selection.allowed_actions.iter().all(|action| {
            action == ENTERPRISE_ATTACHMENT_ACTION || descriptor.actions.contains(action)
        })
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ConnectorAccountId, ConnectorRevision, ConnectorRuntimeKind, ContributionRevision, PluginId,
    };

    use super::*;

    fn selection(actions: &[&str], pinned_account: bool) -> ConnectorRevisionSelection {
        let account_id = ConnectorAccountId::new("account-1");
        ConnectorRevisionSelection {
            account_id: account_id.clone(),
            contribution_revision: ContributionRevision {
                plugin_id: PluginId::new("plugin-1"),
                contribution_id: "connector-1".into(),
                account_id: pinned_account.then_some(account_id),
                content_hash: "content".into(),
                host_identity_hash: Some("host".into()),
                schema_hash: Some("schema".into()),
                action_hash: Some("actions".into()),
            },
            allowed_actions: actions.iter().map(ToString::to_string).collect(),
        }
    }

    fn descriptor() -> ConnectorDriverDescriptor {
        ConnectorDriverDescriptor {
            plugin_id: PluginId::new("plugin-1"),
            connector_id: "connector-1".into(),
            runtime_kind: ConnectorRuntimeKind::Builtin,
            revision: ConnectorRevision {
                host_identity_hash: "host".into(),
                schema_hash: "schema".into(),
                action_hash: "actions".into(),
            },
            actions: vec!["send".into()],
        }
    }

    #[test]
    fn reserved_attachment_action_requires_an_exact_account_pin() {
        assert!(selection_matches_descriptor(
            &selection(&[ENTERPRISE_ATTACHMENT_ACTION], true),
            &descriptor()
        ));
        assert!(!selection_matches_descriptor(
            &selection(&[ENTERPRISE_ATTACHMENT_ACTION], false),
            &descriptor()
        ));
    }

    #[test]
    fn reserved_attachment_action_does_not_widen_generic_connector_actions() {
        assert!(selection_matches_descriptor(
            &selection(&["send", ENTERPRISE_ATTACHMENT_ACTION], true),
            &descriptor()
        ));
        assert!(!selection_matches_descriptor(
            &selection(&["delete"], true),
            &descriptor()
        ));
    }

    #[test]
    fn connector_snapshot_is_compiled_from_the_unified_policy() {
        let mut policy = AgentPermissionPolicy::default();
        policy
            .rules
            .connectors
            .push(hachimi_protocol::ConnectorPermissionRule {
                account_id: ConnectorAccountId::new("account-1"),
                actions: vec!["send".into()],
                read_only_actions: vec!["send".into()],
                contribution_revision: "actions".into(),
            });

        let snapshot =
            snapshot_from_permission_policy(&policy, &[selection(&["send", "delete"], true)])
                .expect("restricted snapshot");
        assert_eq!(snapshot.connectors[0].allowed_actions, ["send"]);

        policy.level = PermissionProfile::FullAccess;
        assert!(snapshot_from_permission_policy(&policy, &[]).is_none());
    }
}
