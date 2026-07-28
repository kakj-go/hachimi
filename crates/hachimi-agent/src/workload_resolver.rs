// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/session/turn_context.rs and
// core-skills/src/{injection,service}.rs at commit
// 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: Workbench workload overlays, cached strict classification,
// Built-in trust boundaries, and General fail-closed fallback.

use std::sync::Arc;

use hachimi_protocol::{
    SkillClassification, SkillScope, WorkloadKind, WorkloadResolution, WorkloadResolutionSource,
};
use hachimi_skills::SkillRunSelection;
use hachimi_storage::AgentStore;
use tokio_util::sync::CancellationToken;

use crate::{ModelRuntime, WorkloadClassificationRequest};

pub const WORKLOAD_CLASSIFIER_REVISION: &str = "hachimi-workload-v1";
const MIN_CONFIDENCE_BASIS_POINTS: u16 = 7_500;
const MAX_CLASSIFIER_SKILL_BYTES: usize = 16 * 1024;

pub async fn resolve_workload(
    override_workload: Option<WorkloadKind>,
    prompt: &str,
    selected: &[SkillRunSelection],
    model: Arc<dyn ModelRuntime>,
    store: &AgentStore,
    cancellation: CancellationToken,
) -> WorkloadResolution {
    let activated_skill_ids = selected
        .iter()
        .map(|selection| selection.record.id.clone())
        .collect::<Vec<_>>();
    if let Some(workload) = override_workload {
        return WorkloadResolution {
            workload,
            source: WorkloadResolutionSource::UserOverride,
            activated_skill_ids,
            reason: "explicit workload override".into(),
            classifier_revision: None,
        };
    }

    let mut trusted = Vec::new();
    for selection in selected {
        let classification = classification_for_selection(
            selection,
            prompt,
            Arc::clone(&model),
            store,
            cancellation.child_token(),
        )
        .await;
        if let Some(classification) = classification
            && classification.confidence_basis_points >= MIN_CONFIDENCE_BASIS_POINTS
            && classification.workload != WorkloadKind::General
        {
            trusted.push(classification);
        }
    }
    trusted.sort_by_key(|classification| classification.workload as u8);
    trusted.dedup_by_key(|classification| classification.workload);
    if trusted.len() == 1 {
        let classification = &trusted[0];
        let built_in_office = selected.iter().any(|selection| {
            selection.record.scope == SkillScope::BuiltIn
                && selection.record.policy.workload == Some(WorkloadKind::Office)
        });
        return WorkloadResolution {
            workload: classification.workload,
            source: if built_in_office && classification.workload == WorkloadKind::Office {
                WorkloadResolutionSource::BuiltInSkill
            } else {
                WorkloadResolutionSource::ExplicitSkill
            },
            activated_skill_ids,
            reason: classification.reason.clone(),
            classifier_revision: Some(classification.classifier_revision.clone()),
        };
    }

    let capabilities = model.capabilities();
    if capabilities.strict_json_schema && capabilities.output_schema {
        let request = WorkloadClassificationRequest {
            prompt: bounded(prompt, MAX_CLASSIFIER_SKILL_BYTES),
            skill_name: None,
            skill_description: Some(
                selected
                    .iter()
                    .map(|selection| {
                        format!(
                            "{}: {}",
                            selection.record.qualified_name, selection.record.description
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            bounded_skill_markdown: None,
            classifier_revision: WORKLOAD_CLASSIFIER_REVISION.into(),
        };
        if let Ok(result) = model.classify_workload(request, cancellation).await
            && result.classifier_revision == WORKLOAD_CLASSIFIER_REVISION
            && result.confidence_basis_points >= MIN_CONFIDENCE_BASIS_POINTS
        {
            return WorkloadResolution {
                workload: result.workload,
                source: WorkloadResolutionSource::StructuredClassification,
                activated_skill_ids,
                reason: result.reason,
                classifier_revision: Some(result.classifier_revision),
            };
        }
    }

    WorkloadResolution {
        workload: WorkloadKind::General,
        source: WorkloadResolutionSource::GeneralFallback,
        activated_skill_ids,
        reason: "strict workload classification was unavailable, conflicting, or below confidence threshold".into(),
        classifier_revision: None,
    }
}

pub async fn classification_for_selection(
    selection: &SkillRunSelection,
    prompt: &str,
    model: Arc<dyn ModelRuntime>,
    store: &AgentStore,
    cancellation: CancellationToken,
) -> Option<SkillClassification> {
    if selection.record.scope == SkillScope::BuiltIn {
        return selection
            .record
            .policy
            .workload
            .map(|workload| SkillClassification {
                skill_id: selection.record.id.clone(),
                content_revision: selection.revision.clone(),
                workload,
                confidence_basis_points: 10_000,
                reason: "trusted Built-in Skill workload metadata".into(),
                classifier_revision: "builtin-metadata-v1".into(),
                classified_at_ms: now_ms(),
            });
    }
    if let Ok(Some(classification)) = store
        .get_skill_classification(&selection.record.id, &selection.revision)
        .await
    {
        return Some(classification);
    }
    let capabilities = model.capabilities();
    if !capabilities.strict_json_schema || !capabilities.output_schema {
        return None;
    }
    let request = WorkloadClassificationRequest {
        prompt: bounded(prompt, MAX_CLASSIFIER_SKILL_BYTES),
        skill_name: Some(selection.record.qualified_name.clone()),
        skill_description: Some(selection.record.description.clone()),
        bounded_skill_markdown: Some(bounded(&selection.instructions, MAX_CLASSIFIER_SKILL_BYTES)),
        classifier_revision: WORKLOAD_CLASSIFIER_REVISION.into(),
    };
    let result = model.classify_workload(request, cancellation).await.ok()?;
    if result.classifier_revision != WORKLOAD_CLASSIFIER_REVISION {
        return None;
    }
    let classification = SkillClassification {
        skill_id: selection.record.id.clone(),
        content_revision: selection.revision.clone(),
        workload: result.workload,
        confidence_basis_points: result.confidence_basis_points.min(10_000),
        reason: bounded(&result.reason, 1_000),
        classifier_revision: result.classifier_revision,
        classified_at_ms: now_ms(),
    };
    store.put_skill_classification(&classification).await.ok()?;
    Some(classification)
}

fn bounded(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use futures_util::stream;
    use hachimi_protocol::{
        ModelRequest, ProviderCapabilities, SkillActivationSource, SkillId, SkillPolicy,
        SkillRecord,
    };
    use hachimi_storage::{AgentStore, StoredSkillRecord};

    use super::*;
    use crate::{ModelEventStream, WorkloadClassificationFuture, WorkloadClassificationResult};

    struct ClassifierModel {
        strict: bool,
        result: WorkloadClassificationResult,
        calls: Arc<AtomicUsize>,
    }

    impl ModelRuntime for ClassifierModel {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                strict_json_schema: self.strict,
                output_schema: self.strict,
                ..ProviderCapabilities::default()
            }
        }

        fn stream(
            &self,
            _request: ModelRequest,
            _cancellation: CancellationToken,
        ) -> ModelEventStream {
            Box::pin(stream::empty())
        }

        fn classify_workload(
            &self,
            _request: WorkloadClassificationRequest,
            _cancellation: CancellationToken,
        ) -> WorkloadClassificationFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let result = self.result.clone();
            Box::pin(async move { Ok(result) })
        }
    }

    async fn selection(scope: SkillScope) -> (AgentStore, SkillRunSelection) {
        let store = AgentStore::connect_in_memory().await.expect("store");
        let record = SkillRecord {
            id: SkillId::random(),
            scope,
            namespace: None,
            name: "document-helper".into(),
            qualified_name: "document-helper".into(),
            description: "Create a document".into(),
            interface: None,
            policy: SkillPolicy {
                allow_implicit_invocation: Some(true),
                workload: (scope == SkillScope::BuiltIn).then_some(WorkloadKind::Office),
            },
            dependencies: Vec::new(),
            editable: scope == SkillScope::User,
            enabled: true,
            content_hash: "revision-one".into(),
            tree_revision: "revision-one".into(),
            diagnostics: Vec::new(),
            updated_at_ms: now_ms(),
        };
        store
            .upsert_skill(&StoredSkillRecord {
                stable_path: "C:\\skills\\document-helper".into(),
                record: record.clone(),
            })
            .await
            .expect("persist Skill");
        (
            store,
            SkillRunSelection {
                record,
                instructions: "Create and validate the document.".into(),
                revision: "revision-one".into(),
                source: SkillActivationSource::ExplicitSelection,
            },
        )
    }

    fn model(
        strict: bool,
        workload: WorkloadKind,
        confidence: u16,
        calls: Arc<AtomicUsize>,
    ) -> Arc<dyn ModelRuntime> {
        Arc::new(ClassifierModel {
            strict,
            result: WorkloadClassificationResult {
                workload,
                confidence_basis_points: confidence,
                reason: "structured test classification".into(),
                classifier_revision: WORKLOAD_CLASSIFIER_REVISION.into(),
            },
            calls,
        })
    }

    #[test]
    fn bounded_markdown_preserves_utf8_boundaries() {
        assert_eq!(bounded("你好", 4), "你");
    }

    #[tokio::test]
    async fn user_skill_without_strict_classifier_stays_general() {
        let (store, selected) = selection(SkillScope::User).await;
        let resolved = resolve_workload(
            None,
            "Create a report",
            &[selected],
            model(
                false,
                WorkloadKind::Office,
                10_000,
                Arc::new(AtomicUsize::new(0)),
            ),
            &store,
            CancellationToken::new(),
        )
        .await;
        assert_eq!(resolved.workload, WorkloadKind::General);
        assert_eq!(resolved.source, WorkloadResolutionSource::GeneralFallback);
    }

    #[tokio::test]
    async fn strict_user_classification_is_cached_by_content_revision() {
        let (store, selected) = selection(SkillScope::User).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let classifier = model(true, WorkloadKind::Office, 9_500, Arc::clone(&calls));
        let first = classification_for_selection(
            &selected,
            "Create a report",
            Arc::clone(&classifier),
            &store,
            CancellationToken::new(),
        )
        .await
        .expect("classification");
        let second = classification_for_selection(
            &selected,
            "A different prompt",
            classifier,
            &store,
            CancellationToken::new(),
        )
        .await
        .expect("cached classification");
        assert_eq!(first, second);
        assert_eq!(first.workload, WorkloadKind::Office);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn low_confidence_classification_and_conflicting_skills_do_not_force_a_domain() {
        let (store, selected) = selection(SkillScope::User).await;
        let resolved = resolve_workload(
            None,
            "Ambiguous task",
            &[selected],
            model(
                true,
                WorkloadKind::Coding,
                7_000,
                Arc::new(AtomicUsize::new(0)),
            ),
            &store,
            CancellationToken::new(),
        )
        .await;
        assert_eq!(resolved.workload, WorkloadKind::General);
    }

    #[tokio::test]
    async fn override_beats_trusted_built_in_office_metadata() {
        let (store, selected) = selection(SkillScope::BuiltIn).await;
        let resolved = resolve_workload(
            Some(WorkloadKind::Coding),
            "Create a document",
            &[selected],
            model(
                false,
                WorkloadKind::Office,
                10_000,
                Arc::new(AtomicUsize::new(0)),
            ),
            &store,
            CancellationToken::new(),
        )
        .await;
        assert_eq!(resolved.workload, WorkloadKind::Coding);
        assert_eq!(resolved.source, WorkloadResolutionSource::UserOverride);
    }
}
