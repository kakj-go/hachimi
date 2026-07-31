use hachimi_protocol::{
    ConnectorDriverDescriptor, ScheduleConnectorSelection, ScheduleDefinition, WorkloadKind,
};

pub(super) const ENTERPRISE_ATTACHMENT_ACTION: &str = "download_attachment";
pub(super) const ENTERPRISE_ATTACHMENT_TOOL: &str = "enterprise.download_attachment";
pub(super) const SCHEDULE_HOST_GRANT_ATTENTION_EVENT: &str = "schedule.host_grant.needs_attention";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScheduleHostGrantError {
    pub(super) code: &'static str,
    pub(super) message: String,
}

impl ScheduleHostGrantError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            code: "schedule_enterprise_attachment_not_authorized",
            message: message.into(),
        }
    }

    fn drift(message: impl Into<String>) -> Self {
        Self {
            code: "schedule_connector_action_drift",
            message: message.into(),
        }
    }
}

pub(super) fn validate_enterprise_attachment_scope(
    schedule: &ScheduleDefinition,
) -> Result<(), ScheduleHostGrantError> {
    if !schedule
        .tool_allowlist
        .iter()
        .any(|tool| tool == ENTERPRISE_ATTACHMENT_TOOL)
    {
        return Ok(());
    }
    if schedule.workload_override == Some(WorkloadKind::Coding) {
        return Err(ScheduleHostGrantError::unauthorized(
            "scheduled enterprise attachment downloads are limited to General and Office workloads",
        ));
    }
    if !schedule
        .host_grant
        .connectors
        .iter()
        .any(attachment_selection_is_exact)
    {
        return Err(ScheduleHostGrantError::unauthorized(
            "enterprise.download_attachment requires a pinned Connector account, contribution revision, and download_attachment action",
        ));
    }
    Ok(())
}

pub(super) async fn validate_schedule_connector_selections(
    host: &hachimi_extensions::PluginHost,
    selections: &[ScheduleConnectorSelection],
) -> Result<(), ScheduleHostGrantError> {
    let revisions = selections
        .iter()
        .map(|selection| selection.contribution_revision.clone())
        .collect::<Vec<_>>();
    host.verify_contribution_revisions(&revisions)
        .await
        .map_err(|error| {
            ScheduleHostGrantError::drift(format!(
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
                ScheduleHostGrantError::drift(format!(
                    "Connector driver is unavailable or changed: {error}"
                ))
            })?;
        if !selection_matches_descriptor(selection, &descriptor) {
            return Err(ScheduleHostGrantError::drift(
                "Connector Host identity, schema, action revision, account, or allowed action changed",
            ));
        }
    }
    Ok(())
}

pub(super) async fn authorize_enterprise_attachment_download(
    host: &hachimi_extensions::PluginHost,
    selections: &[ScheduleConnectorSelection],
    integration_account_id: &str,
) -> Result<(), ScheduleHostGrantError> {
    let connector_account_id = host
        .enterprise_attachment_connector_account(integration_account_id)
        .await
        .map_err(|error| {
            ScheduleHostGrantError::drift(format!(
                "enterprise attachment account binding is unavailable: {error}"
            ))
        })?
        .ok_or_else(|| {
            ScheduleHostGrantError::unauthorized(
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
            ScheduleHostGrantError::unauthorized(
                "attachment account or download_attachment action is outside ScheduleHostGrant",
            )
        })?;
    if !attachment_selection_is_exact(selection) {
        return Err(ScheduleHostGrantError::unauthorized(
            "attachment ScheduleHostGrant does not pin the selected account",
        ));
    }
    validate_schedule_connector_selections(host, std::slice::from_ref(selection)).await
}

fn attachment_selection_is_exact(selection: &ScheduleConnectorSelection) -> bool {
    selection
        .allowed_actions
        .iter()
        .any(|action| action == ENTERPRISE_ATTACHMENT_ACTION)
        && selection.contribution_revision.account_id.as_ref() == Some(&selection.account_id)
}

fn selection_matches_descriptor(
    selection: &ScheduleConnectorSelection,
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

    fn selection(actions: &[&str], pinned_account: bool) -> ScheduleConnectorSelection {
        let account_id = ConnectorAccountId::new("account-1");
        ScheduleConnectorSelection {
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
}
