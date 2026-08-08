use std::collections::BTreeMap;

use serde::Serialize;

const MAX_PLAN_CHARS: usize = 8 * 1024;
const MAX_LABEL_CHARS: usize = 512;
const MAX_REFERENCE_CHARS: usize = 1_024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeContinuitySnapshot {
    pub session_id: String,
    pub run_id: String,
    pub run_generation: u64,
    pub host_context: Option<String>,
    pub accepted_plan: Option<ContinuityPlan>,
    pub environment: Option<ContinuityEnvironment>,
    pub workspace: Option<ContinuityWorkspace>,
    pub recent_session_sources: Vec<ContinuitySessionSource>,
    pub artifacts: Vec<ContinuityArtifact>,
    pub connector_revisions: Vec<ContinuityConnectorRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinuityPlan {
    pub id: String,
    pub revision: u32,
    pub status: String,
    pub content_markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinuityEnvironment {
    pub revision: u64,
    pub binding_revision: u64,
    pub baseline_revision: Option<String>,
    pub inactive_head: Option<String>,
    pub inactive_status_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ContinuityWorkspace {
    Workspace {
        workspace_id: String,
    },
    Project {
        project_id: String,
        checkout_id: String,
        checkout_kind: Option<String>,
        checkout_status: Option<String>,
        base_revision: Option<String>,
        head_revision: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinuitySessionSource {
    pub id: String,
    pub origin: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub attachment_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinuityArtifact {
    pub id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub display_name: String,
    pub content_hash: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinuityConnectorRevision {
    pub account_id: String,
    pub contribution_revision: ContinuityContributionRevision,
    pub allowed_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinuityContributionRevision {
    pub plugin_id: String,
    pub contribution_id: String,
    pub content_hash: String,
    pub host_identity_hash: Option<String>,
    pub schema_hash: Option<String>,
    pub action_hash: Option<String>,
}

impl RuntimeContinuitySnapshot {
    pub(crate) fn render(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"error":"runtime_continuity_serialization_failed"}"#.to_owned())
    }
}

pub(crate) fn bounded_plan(value: &str) -> String {
    bounded(value, MAX_PLAN_CHARS)
}

pub(crate) fn bounded_label(value: &str) -> String {
    bounded(value, MAX_LABEL_CHARS)
}

pub(crate) fn bounded_reference(value: &str) -> String {
    bounded(value, MAX_REFERENCE_CHARS)
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_typed_references_without_scanning_transcript_text() {
        let snapshot = RuntimeContinuitySnapshot {
            session_id: "session-typed".into(),
            run_id: "run-typed".into(),
            run_generation: 3,
            accepted_plan: Some(ContinuityPlan {
                id: "plan-typed".into(),
                revision: 4,
                status: "accepted".into(),
                content_markdown: "Do the typed work".into(),
            }),
            artifacts: vec![ContinuityArtifact {
                id: "artifact-typed".into(),
                run_id: Some("run-typed".into()),
                kind: "document".into(),
                display_name: "report.docx".into(),
                content_hash: Some("sha256:abc".into()),
                metadata: BTreeMap::from([("mimeType".into(), "application/docx".into())]),
            }],
            connector_revisions: vec![ContinuityConnectorRevision {
                account_id: "connector-account".into(),
                contribution_revision: ContinuityContributionRevision {
                    plugin_id: "plugin".into(),
                    contribution_id: "connector".into(),
                    content_hash: "revision-7".into(),
                    host_identity_hash: None,
                    schema_hash: None,
                    action_hash: None,
                },
                allowed_actions: vec!["search".into()],
            }],
            ..RuntimeContinuitySnapshot::default()
        };

        let rendered = snapshot.render();
        assert!(rendered.contains("\"acceptedPlan\""));
        assert!(rendered.contains("plan-typed"));
        assert!(rendered.contains("artifact-typed"));
        assert!(rendered.contains("revision-7"));
    }

    #[test]
    fn bounds_plan_and_reference_content() {
        assert_eq!(
            bounded_plan(&"p".repeat(MAX_PLAN_CHARS + 5)).len(),
            MAX_PLAN_CHARS
        );
        assert_eq!(
            bounded_reference(&"r".repeat(MAX_REFERENCE_CHARS + 5)).len(),
            MAX_REFERENCE_CHARS
        );
    }
}
